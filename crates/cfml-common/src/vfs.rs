//! Virtual filesystem abstraction for RustCFML.
//!
//! Allows the VM to read source files from either the real filesystem (`RealFs`)
//! or from an in-memory archive embedded in the binary (`EmbeddedFs`).

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

/// Directory entry returned by `Vfs::read_dir`.
#[derive(Debug, Clone)]
pub struct VfsDirEntry {
    pub name: String,
    pub is_file: bool,
    pub is_dir: bool,
}

/// A streaming reader over one file, yielding one `loop file=` iteration at a
/// time.
///
/// Exists so `loop file=` can bound its memory to a single chunk instead of
/// materialising the whole file (GH #367): the reporter's files run to over a
/// million rows, and the eager path cost file-size + one `String` per line
/// *before the first iteration* — strictly worse than the `fileRead()` +
/// `listToArray()` workaround the construct is supposed to replace.
pub trait VfsFileChunks: Send {
    /// The next chunk, or `None` at end of file.
    fn next_chunk(&mut self) -> io::Result<Option<String>>;
}

/// What one iteration of `loop file=` yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChunking {
    /// One line, `str::lines` semantics: the terminator is stripped, `\r\n`
    /// and `\n` both end a line, a trailing newline does NOT yield a final
    /// empty line, and interior blank lines ARE yielded so line numbers stay
    /// accurate.
    Lines,
    /// Exactly N **characters** (not bytes — a multi-byte character counts
    /// once), terminators included verbatim, with the last chunk holding
    /// whatever is left. This is `<cfloop file= characters=N>`.
    Chars(usize),
}

/// How to read a file for `loop file=`: what a chunk is, and how to decode
/// bytes into characters (`charset=`).
#[derive(Clone, Copy, Debug)]
pub struct FileCursorOpts {
    pub chunking: FileChunking,
    pub charset: crate::charset::Charset,
}

impl Default for FileCursorOpts {
    fn default() -> Self {
        FileCursorOpts {
            chunking: FileChunking::Lines,
            charset: crate::charset::Charset::Utf8,
        }
    }
}

/// Decoded-but-not-yet-yielded text, split into chunks on demand. Shared by
/// the streaming and eager readers so the two cannot disagree about what a
/// chunk is.
///
/// Consumed text is left in place behind a read cursor and dropped once per
/// refill, not once per chunk. Draining from the front per chunk instead
/// memmoves the whole remaining buffer every time — with 16KB buffered and
/// ~40-byte lines that was ~6MB of shifting per block read, and it cost the
/// million-line loop ~10% of its wall clock.
struct ChunkBuf {
    text: String,
    /// Byte offset of the first unconsumed character.
    pos: usize,
}

impl ChunkBuf {
    fn new(text: String) -> Self {
        ChunkBuf { text, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.text.len()
    }

    fn rest(&self) -> &str {
        &self.text[self.pos..]
    }

    /// Append more decoded text, first discarding what has been consumed.
    fn refill_with(&mut self, decoder: &mut crate::charset::StreamDecoder, bytes: &[u8]) {
        self.compact();
        decoder.push_into(bytes, &mut self.text);
    }

    fn refill_finish(&mut self, decoder: &mut crate::charset::StreamDecoder) {
        self.compact();
        let tail = decoder.finish();
        self.text.push_str(&tail);
    }

    fn compact(&mut self) {
        if self.pos > 0 {
            self.text.drain(..self.pos);
            self.pos = 0;
        }
    }

    /// The next chunk, or `None` when more text is needed (`at_eof` false) or
    /// the file is exhausted (`at_eof` true).
    fn take(&mut self, chunking: FileChunking, at_eof: bool) -> Option<String> {
        match chunking {
            FileChunking::Lines => match self.rest().find('\n') {
                Some(nl) => {
                    let end = self.pos + nl;
                    // `\r\n` — strip the carriage return with the newline, as
                    // `str::lines` and `BufRead::lines` both do.
                    let stop = if self.text[self.pos..end].ends_with('\r') { end - 1 } else { end };
                    let line = self.text[self.pos..stop].to_string();
                    self.pos = end + 1;
                    Some(line)
                }
                // A final line with no terminator is still a line; a trailing
                // newline leaves nothing behind, which is why this is not
                // `Some("")`.
                None if at_eof && !self.is_empty() => {
                    let line = self.rest().to_string();
                    self.pos = self.text.len();
                    Some(line)
                }
                None => None,
            },
            FileChunking::Chars(n) => match self.rest().char_indices().nth(n) {
                Some((offset, _)) => {
                    let end = self.pos + offset;
                    let chunk = self.text[self.pos..end].to_string();
                    self.pos = end;
                    Some(chunk)
                }
                // Fewer than N characters buffered: at EOF that is the last
                // (short) chunk, mid-stream it means read more.
                None if at_eof && !self.is_empty() => {
                    let chunk = self.rest().to_string();
                    self.pos = self.text.len();
                    Some(chunk)
                }
                None => None,
            },
        }
    }
}

/// Eager fallback: the whole file decoded up-front, handed back one chunk at a
/// time.
///
/// The default for VFS implementations that have nothing to stream *from* — an
/// embedded archive is already resident in memory, so a "streaming" read of it
/// would save nothing. Callers get the same values either way; only the peak
/// memory differs, and for those implementations it cannot be improved.
pub struct EagerChunks {
    buf: ChunkBuf,
    chunking: FileChunking,
}

impl EagerChunks {
    pub fn new(content: &str, chunking: FileChunking) -> Self {
        EagerChunks { buf: ChunkBuf::new(content.to_string()), chunking }
    }
}

impl VfsFileChunks for EagerChunks {
    fn next_chunk(&mut self) -> io::Result<Option<String>> {
        // Everything is already in hand, so "not enough text yet" cannot
        // happen — at_eof is always true here.
        Ok(self.buf.take(self.chunking, true))
    }
}

/// Virtual filesystem trait — abstracts source file I/O so the VM can read
/// from disk or from an embedded archive.
pub trait Vfs: Send + Sync {
    fn read_to_string(&self, path: &str) -> io::Result<String>;
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
    fn exists(&self, path: &str) -> bool;
    fn is_file(&self, path: &str) -> bool;
    fn is_dir(&self, path: &str) -> bool;
    fn read_dir(&self, path: &str) -> io::Result<Vec<VfsDirEntry>>;
    /// File modification time (for bytecode cache invalidation).
    fn modified(&self, path: &str) -> io::Result<SystemTime>;
    /// Canonicalize a path (resolve symlinks, make absolute).
    fn canonicalize(&self, path: &str) -> io::Result<String>;

    /// Open a file for chunk-by-chunk streaming (see [`VfsFileChunks`]).
    ///
    /// Defaults to reading the file whole and iterating the result, which is
    /// correct for every implementation and optimal for the in-memory ones.
    /// [`RealFs`] overrides it with a buffered reader so a large file on disk
    /// costs one chunk of resident memory rather than its whole size.
    /// Delegating implementations must forward this, or they silently drop
    /// back to the eager path for the files that most need streaming.
    fn open_chunks(
        &self,
        path: &str,
        opts: FileCursorOpts,
    ) -> io::Result<Box<dyn VfsFileChunks>> {
        let text = crate::charset::decode(&self.read(path)?, opts.charset);
        Ok(Box::new(EagerChunks::new(&text, opts.chunking)))
    }
}

// ---------------------------------------------------------------------------
// RealFs — delegates to std::fs (default behavior)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RealFs;

impl Vfs for RealFs {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn is_file(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn is_dir(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }

    fn read_dir(&self, path: &str) -> io::Result<Vec<VfsDirEntry>> {
        let entries = std::fs::read_dir(path)?;
        let mut result = Vec::new();
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                // Classify by following symlinks (Lucee parity): a symlink to a
                // directory must report is_dir=true so recursive listings descend
                // into it. DirEntry::metadata() returns the link's own metadata
                // (like symlink_metadata) — fs::metadata() traverses the link.
                let (is_file, is_dir) = std::fs::metadata(entry.path())
                    .map(|md| (md.is_file(), md.is_dir()))
                    .unwrap_or((false, false));
                result.push(VfsDirEntry {
                    name: name.to_string(),
                    is_file,
                    is_dir,
                });
            }
        }
        Ok(result)
    }

    fn modified(&self, path: &str) -> io::Result<SystemTime> {
        std::fs::metadata(path)?.modified()
    }

    /// Buffered, one chunk resident at a time — this is the whole point of
    /// [`Vfs::open_chunks`]. Chunk boundaries and decoding both go through the
    /// same helpers the eager path uses ([`take_chunk`],
    /// [`crate::charset::StreamDecoder`], which is asserted equivalent to
    /// `charset::decode` of the whole file), so the values are identical
    /// either way and only peak memory differs.
    fn open_chunks(
        &self,
        path: &str,
        opts: FileCursorOpts,
    ) -> io::Result<Box<dyn VfsFileChunks>> {
        let file = std::fs::File::open(path)?;
        Ok(Box::new(StreamFileChunks {
            reader: io::BufReader::new(file),
            decoder: crate::charset::StreamDecoder::new(opts.charset),
            buf: ChunkBuf::new(String::new()),
            chunking: opts.chunking,
            at_eof: false,
        }))
    }

    fn canonicalize(&self, path: &str) -> io::Result<String> {
        std::fs::canonicalize(path).map(|p| p.to_string_lossy().to_string())
    }
}

// ---------------------------------------------------------------------------
// EmbeddedFs — reads from an in-memory archive
// ---------------------------------------------------------------------------

/// An in-memory filesystem backed by a map of normalized paths to file contents.
/// All paths are stored as forward-slash-separated, lowercase, without leading slash.
pub struct EmbeddedFs {
    /// Normalized path → file contents
    files: HashMap<String, Vec<u8>>,
    /// Normalized directory paths that exist (computed from file paths)
    dirs: std::collections::HashSet<String>,
    /// The base directory that was embedded (used for canonicalize)
    base_dir: String,
    /// Fixed mtime for all embedded files (set at build time)
    mtime: SystemTime,
}

impl std::fmt::Debug for EmbeddedFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedFs")
            .field("file_count", &self.files.len())
            .field("base_dir", &self.base_dir)
            .finish()
    }
}

impl EmbeddedFs {
    /// Create from a map of relative paths to file contents.
    /// Paths should use forward slashes and be relative to the app root.
    pub fn new(files: HashMap<String, Vec<u8>>, base_dir: String) -> Self {
        let mut dirs = std::collections::HashSet::new();
        // Normalize all file keys to lowercase for case-insensitive lookup
        let mut normalized_files = HashMap::new();
        for (path, data) in files {
            let normalized = Self::normalize_path_static(&path);
            let mut current = String::new();
            for segment in normalized.split('/') {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(segment);
                // Don't add the file itself as a dir
                if current != normalized {
                    dirs.insert(current.clone());
                }
            }
            // Also add the root dir
            dirs.insert(String::new());
            normalized_files.insert(normalized, data);
        }
        Self {
            files: normalized_files,
            dirs,
            base_dir,
            mtime: crate::clock::now_system_time(),
        }
    }

    fn normalize_path_static(path: &str) -> String {
        // Strip base_dir prefix if present, normalize separators and case
        path.replace('\\', "/")
            .trim_start_matches('/')
            .to_lowercase()
    }

    /// Normalize a path: resolve relative to base_dir, strip prefix, lowercase
    fn normalize(&self, path: &str) -> String {
        let path = path.replace('\\', "/");

        // If it starts with the base_dir, strip it
        let stripped = if !self.base_dir.is_empty() {
            let base_lower = self.base_dir.replace('\\', "/").to_lowercase();
            let path_lower = path.to_lowercase();
            if path_lower.starts_with(&base_lower) {
                let remainder = &path[self.base_dir.len()..];
                remainder.trim_start_matches('/').to_lowercase()
            } else {
                path.trim_start_matches('/').to_lowercase()
            }
        } else {
            path.trim_start_matches('/').to_lowercase()
        };

        // Clean up . and .. segments
        let mut parts: Vec<&str> = Vec::new();
        for segment in stripped.split('/') {
            match segment {
                "." | "" => {}
                ".." => { parts.pop(); }
                s => parts.push(s),
            }
        }
        parts.join("/")
    }
}

/// [`RealFs`]'s streaming reader — a `BufReader` over the open file, decoded
/// incrementally, with only the current chunk (plus at most one read block)
/// resident.
struct StreamFileChunks {
    reader: io::BufReader<std::fs::File>,
    decoder: crate::charset::StreamDecoder,
    /// Decoded text not yet handed out. Never larger than one chunk plus one
    /// read block.
    buf: ChunkBuf,
    chunking: FileChunking,
    at_eof: bool,
}

/// Bytes read per refill. Large enough that a line-at-a-time loop does not
/// syscall per line, small enough to stay irrelevant next to the engine's
/// baseline footprint.
const CHUNK_READ_BLOCK: usize = 16 * 1024;

impl VfsFileChunks for StreamFileChunks {
    fn next_chunk(&mut self) -> io::Result<Option<String>> {
        loop {
            if let Some(chunk) = self.buf.take(self.chunking, self.at_eof) {
                return Ok(Some(chunk));
            }
            if self.at_eof {
                return Ok(None);
            }
            let mut block = [0u8; CHUNK_READ_BLOCK];
            let read = io::Read::read(&mut self.reader, &mut block)?;
            if read == 0 {
                self.at_eof = true;
                // A character left half-decoded by a truncated file becomes
                // U+FFFD here rather than being dropped.
                self.buf.refill_finish(&mut self.decoder);
            } else {
                self.buf.refill_with(&mut self.decoder, &block[..read]);
            }
        }
    }
}

impl Vfs for EmbeddedFs {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        let normalized = self.normalize(path);
        self.files.get(&normalized)
            .map(|data| String::from_utf8_lossy(data).to_string())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,
                format!("embedded file not found: {} (normalized: {})", path, normalized)))
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let normalized = self.normalize(path);
        self.files.get(&normalized)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,
                format!("embedded file not found: {} (normalized: {})", path, normalized)))
    }

    fn exists(&self, path: &str) -> bool {
        let normalized = self.normalize(path);
        self.files.contains_key(&normalized) || self.dirs.contains(&normalized)
    }

    fn is_file(&self, path: &str) -> bool {
        let normalized = self.normalize(path);
        self.files.contains_key(&normalized)
    }

    fn is_dir(&self, path: &str) -> bool {
        let normalized = self.normalize(path);
        self.dirs.contains(&normalized)
    }

    fn read_dir(&self, path: &str) -> io::Result<Vec<VfsDirEntry>> {
        let normalized = self.normalize(path);
        if !self.dirs.contains(&normalized) {
            return Err(io::Error::new(io::ErrorKind::NotFound,
                format!("embedded directory not found: {}", path)));
        }

        let prefix = if normalized.is_empty() {
            String::new()
        } else {
            format!("{}/", normalized)
        };

        let mut seen = std::collections::HashSet::new();
        let mut entries = Vec::new();

        // Find direct children (files and dirs)
        for file_path in self.files.keys() {
            if file_path.starts_with(&prefix) {
                let remainder = &file_path[prefix.len()..];
                // Direct child: no more slashes
                if let Some(slash_pos) = remainder.find('/') {
                    // It's a subdirectory entry
                    let dir_name = &remainder[..slash_pos];
                    if seen.insert(dir_name.to_string()) {
                        entries.push(VfsDirEntry {
                            name: dir_name.to_string(),
                            is_file: false,
                            is_dir: true,
                        });
                    }
                } else {
                    // Direct file child
                    entries.push(VfsDirEntry {
                        name: remainder.to_string(),
                        is_file: true,
                        is_dir: false,
                    });
                }
            }
        }

        Ok(entries)
    }

    fn modified(&self, path: &str) -> io::Result<SystemTime> {
        let normalized = self.normalize(path);
        if self.files.contains_key(&normalized) {
            Ok(self.mtime)
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "file not found"))
        }
    }

    fn canonicalize(&self, path: &str) -> io::Result<String> {
        // For embedded fs, return the path joined with base_dir
        let normalized = self.normalize(path);
        if self.files.contains_key(&normalized) || self.dirs.contains(&normalized) {
            if self.base_dir.is_empty() {
                Ok(format!("/{}", normalized))
            } else {
                Ok(format!("{}/{}", self.base_dir, normalized))
            }
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound,
                format!("cannot canonicalize: {}", path)))
        }
    }
}

// ---------------------------------------------------------------------------
// FallbackFs — tries embedded FS first, falls back to real filesystem.
// Used in embedded binaries so they can load external files (e.g. modules).
// ---------------------------------------------------------------------------

pub struct FallbackFs {
    pub embedded: EmbeddedFs,
    pub real: RealFs,
    /// When true, only the embedded FS is used (no disk fallback).
    /// Set this in sandbox mode to prevent filesystem access.
    pub sandbox: bool,
}

impl std::fmt::Debug for FallbackFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackFs")
            .field("embedded", &self.embedded)
            .finish()
    }
}

impl Vfs for FallbackFs {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        let result = self.embedded.read_to_string(path);
        if result.is_ok() || self.sandbox { return result; }
        self.real.read_to_string(path)
    }
    /// Forwarded, not defaulted — the whole point of a `--build` binary reading
    /// a large data file off disk is that it streams. Resolution order mirrors
    /// `read_to_string`: embedded first, then the real FS unless sandboxed.
    fn open_chunks(
        &self,
        path: &str,
        opts: FileCursorOpts,
    ) -> io::Result<Box<dyn VfsFileChunks>> {
        let result = self.embedded.open_chunks(path, opts);
        if result.is_ok() || self.sandbox { return result; }
        self.real.open_chunks(path, opts)
    }
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let result = self.embedded.read(path);
        if result.is_ok() || self.sandbox { return result; }
        self.real.read(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.embedded.exists(path) || (!self.sandbox && self.real.exists(path))
    }
    fn is_file(&self, path: &str) -> bool {
        self.embedded.is_file(path) || (!self.sandbox && self.real.is_file(path))
    }
    fn is_dir(&self, path: &str) -> bool {
        self.embedded.is_dir(path) || (!self.sandbox && self.real.is_dir(path))
    }
    fn read_dir(&self, path: &str) -> io::Result<Vec<VfsDirEntry>> {
        if self.sandbox { return self.embedded.read_dir(path); }
        // Merge real FS and embedded FS listings so embedded files (e.g.
        // Application.cfc) remain visible even when the real directory also
        // exists (e.g. the compiled binary's own CWD contains real files
        // alongside the embedded app). Real-FS entries take precedence over
        // embedded ones on a case-insensitive name collision, preserving the
        // intended behaviour of letting on-disk modules override embedded ones.
        let real = self.real.read_dir(path);
        let embedded = self.embedded.read_dir(path);
        match (real, embedded) {
            (Ok(mut real_entries), Ok(embedded_entries)) => {
                let real_names: std::collections::HashSet<String> =
                    real_entries.iter().map(|e| e.name.to_lowercase()).collect();
                for entry in embedded_entries {
                    if !real_names.contains(&entry.name.to_lowercase()) {
                        real_entries.push(entry);
                    }
                }
                Ok(real_entries)
            }
            (Ok(real_entries), Err(_)) => Ok(real_entries),
            (Err(_), Ok(embedded_entries)) => Ok(embedded_entries),
            (Err(e), Err(_)) => Err(e),
        }
    }
    fn modified(&self, path: &str) -> io::Result<SystemTime> {
        let result = self.embedded.modified(path);
        if result.is_ok() || self.sandbox { return result; }
        self.real.modified(path)
    }
    fn canonicalize(&self, path: &str) -> io::Result<String> {
        let result = self.embedded.canonicalize(path);
        if result.is_ok() || self.sandbox { return result; }
        self.real.canonicalize(path)
    }
}

// ---------------------------------------------------------------------------
// Archive format for embedding files in the binary
// ---------------------------------------------------------------------------

/// Magic bytes appended at the very end of a self-contained binary.
pub const ARCHIVE_MAGIC: &[u8; 5] = b"RCFML";

/// Serialize a file map into a binary archive.
/// Format: [file_count:u32] [path_len:u32 path_bytes data_len:u32 data_bytes]...
pub fn serialize_archive(files: &HashMap<String, Vec<u8>>) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (path, data) in files {
        let path_bytes = path.as_bytes();
        buf.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(path_bytes);
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(data);
    }
    buf
}

/// Deserialize a binary archive into a file map.
pub fn deserialize_archive(data: &[u8]) -> io::Result<HashMap<String, Vec<u8>>> {
    let mut pos = 0;
    if data.len() < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "archive too small"));
    }
    let file_count = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    let mut files = HashMap::with_capacity(file_count);
    for _ in 0..file_count {
        if pos + 4 > data.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated archive"));
        }
        let path_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + path_len > data.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated path"));
        }
        let path = String::from_utf8_lossy(&data[pos..pos + path_len]).to_string();
        pos += path_len;

        if pos + 4 > data.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated archive"));
        }
        let data_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + data_len > data.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated file data"));
        }
        let file_data = data[pos..pos + data_len].to_vec();
        pos += data_len;

        files.insert(path, file_data);
    }
    Ok(files)
}

/// Check if the current binary has an embedded archive and extract it.
/// Binary layout: [original_binary][archive_data][archive_len:u64][RCFML]
pub fn extract_embedded_archive() -> Option<HashMap<String, Vec<u8>>> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_data = std::fs::read(&exe_path).ok()?;
    extract_archive_from_bytes(&exe_data)
}

/// Extract archive from raw binary bytes (testable without exe).
///
/// On macOS, `codesign` may append a code signature after our archive trailer,
/// so we scan backwards (up to 64KB) for the RCFML magic bytes.
pub fn extract_archive_from_bytes(data: &[u8]) -> Option<HashMap<String, Vec<u8>>> {
    let len = data.len();
    let min_size = ARCHIVE_MAGIC.len() + 8;
    if len < min_size {
        return None;
    }

    // Scan backwards for RCFML magic (code signature may follow it).
    // macOS code signatures scale with binary size (~8 bytes per 4KB page
    // for SHA-256 hashes, plus overhead). Use 5% of binary size or 1MB,
    // whichever is larger, to handle any realistic binary.
    let scan_window = (len / 20).max(1024 * 1024);
    let scan_limit = len.saturating_sub(scan_window).max(min_size);
    let mut magic_start = None;
    let mut pos = len - ARCHIVE_MAGIC.len();
    while pos >= scan_limit {
        if &data[pos..pos + ARCHIVE_MAGIC.len()] == ARCHIVE_MAGIC.as_slice() {
            magic_start = Some(pos);
            break;
        }
        if pos == 0 { break; }
        pos -= 1;
    }
    let magic_start = magic_start?;

    // Read archive length (u64 LE before magic)
    if magic_start < 8 {
        return None;
    }
    let len_start = magic_start - 8;
    let archive_len = u64::from_le_bytes(data[len_start..len_start + 8].try_into().ok()?) as usize;

    // Extract archive data
    if archive_len > len_start {
        return None;
    }
    let archive_start = len_start - archive_len;
    let archive_data = &data[archive_start..len_start];
    deserialize_archive(archive_data).ok()
}

/// Create a self-contained binary: append archive + length + magic to the base binary.
pub fn create_self_contained_binary(
    base_binary: &[u8],
    files: &HashMap<String, Vec<u8>>,
) -> Vec<u8> {
    let archive = serialize_archive(files);
    let archive_len = archive.len() as u64;

    let mut output = Vec::with_capacity(base_binary.len() + archive.len() + 8 + ARCHIVE_MAGIC.len());
    output.extend_from_slice(base_binary);
    output.extend_from_slice(&archive);
    output.extend_from_slice(&archive_len.to_le_bytes());
    output.extend_from_slice(ARCHIVE_MAGIC);
    output
}

/// Default VFS instance (real filesystem).
pub fn real_fs() -> Arc<dyn Vfs> {
    Arc::new(RealFs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory on the real filesystem, removed on drop. Each call
    /// gets a unique path (pid + monotonic counter) so tests don't collide.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("rustcfml-vfs-test-{}-{}-{}", tag, std::process::id(), n));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
        fn str(&self) -> &str { self.0.to_str().unwrap() }
        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.0.join(name), contents).expect("write temp file");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry_names_lower(entries: &[VfsDirEntry]) -> Vec<String> {
        let mut v: Vec<String> = entries.iter().map(|e| e.name.to_lowercase()).collect();
        v.sort();
        v
    }

    fn fallback_for(real_dir: &str, embedded_files: &[(&str, &str)]) -> FallbackFs {
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        for (path, body) in embedded_files {
            files.insert(path.to_string(), body.as_bytes().to_vec());
        }
        FallbackFs {
            embedded: EmbeddedFs::new(files, real_dir.to_string()),
            real: RealFs,
            sandbox: false,
        }
    }

    /// The compiled-binary regression: the real CWD exists and holds unrelated
    /// files, while the app (incl. Application.cfc) lives only in the embedded
    /// VFS under that same base dir. `read_dir` MUST surface the embedded files
    /// or `find_application_cfc`'s tree-walk never finds Application.cfc and the
    /// binary 500s on every request. (Was broken from v0.6.2 through v0.243.0.)
    #[test]
    fn read_dir_merges_embedded_app_into_real_cwd() {
        let dir = TempDir::new("merge");
        // Real filesystem holds the binary + sundry files, but NO Application.cfc.
        dir.write("myapp", "binary bytes");
        dir.write("notes.txt", "todo");

        let fs = fallback_for(
            dir.str(),
            &[
                ("Application.cfc", "component {}"),
                ("index.cfm", "hi"),
                ("modules/home/handler.cfc", "component {}"),
            ],
        );

        let entries = fs.read_dir(dir.str()).expect("read_dir should succeed");
        let names = entry_names_lower(&entries);

        // Real-FS entries are still present.
        assert!(names.contains(&"myapp".to_string()), "real files preserved: {names:?}");
        assert!(names.contains(&"notes.txt".to_string()), "real files preserved: {names:?}");
        // The embedded-only app files are now visible — this is the fix.
        assert!(names.contains(&"application.cfc".to_string()), "Application.cfc must be visible: {names:?}");
        assert!(names.contains(&"index.cfm".to_string()), "index.cfm must be visible: {names:?}");
        assert!(names.contains(&"modules".to_string()), "embedded subdir must be visible: {names:?}");

        // And the embedded subdirectory listing works too (modules.home resolution).
        let modules = fs.read_dir(&format!("{}/modules", dir.str())).expect("read modules");
        assert!(entry_names_lower(&modules).contains(&"home".to_string()));
    }

    /// On a case-insensitive name collision, the real-FS entry wins (so on-disk
    /// modules can override embedded ones) and the name is NOT duplicated.
    #[test]
    fn read_dir_real_wins_on_collision() {
        let dir = TempDir::new("collision");
        dir.write("shared.cfc", "real version");

        let fs = fallback_for(dir.str(), &[("shared.cfc", "embedded version"), ("only_embedded.cfm", "x")]);

        let entries = fs.read_dir(dir.str()).expect("read_dir");
        let shared: Vec<&VfsDirEntry> = entries
            .iter()
            .filter(|e| e.name.eq_ignore_ascii_case("shared.cfc"))
            .collect();
        assert_eq!(shared.len(), 1, "collision must not duplicate the entry: {entries:?}");
        // Real FS preserves original casing ("shared.cfc"); the embedded copy is
        // lowercased, so the surviving entry being exactly "shared.cfc" proves
        // the real entry won.
        assert_eq!(shared[0].name, "shared.cfc", "real-FS entry must win the collision");
        // Embedded-only entry still comes through.
        assert!(entry_names_lower(&entries).contains(&"only_embedded.cfm".to_string()));
    }

    /// Sandbox mode never touches the real filesystem, even when it exists.
    #[test]
    fn read_dir_sandbox_is_embedded_only() {
        let dir = TempDir::new("sandbox");
        dir.write("myapp", "binary");
        dir.write("notes.txt", "todo");

        let mut fs = fallback_for(dir.str(), &[("index.cfm", "hi")]);
        fs.sandbox = true;

        let names = entry_names_lower(&fs.read_dir(dir.str()).expect("read_dir"));
        assert_eq!(names, vec!["index.cfm".to_string()], "sandbox must hide real FS: {names:?}");
    }

    /// When the real directory doesn't exist at all (read fails), the embedded
    /// listing still comes through — the original `or_else` fallback case.
    #[test]
    fn read_dir_embedded_only_when_real_missing() {
        let base = format!("{}/does-not-exist-{}", std::env::temp_dir().display(), std::process::id());
        let fs = fallback_for(&base, &[("Application.cfc", "component {}"), ("index.cfm", "hi")]);

        let names = entry_names_lower(&fs.read_dir(&base).expect("embedded read_dir"));
        assert!(names.contains(&"application.cfc".to_string()), "{names:?}");
        assert!(names.contains(&"index.cfm".to_string()), "{names:?}");
    }
}

/// The system temp directory WITH a trailing separator, which is what Lucee and
/// Adobe CF both return from `getTempDirectory()`.
///
/// `std::env::temp_dir()` trails on macOS (TMPDIR happens to) but not on Linux,
/// where a bare `/tmp` turned the ubiquitous `getTempDirectory() & name` join
/// into `/tmpname` — a path at the filesystem ROOT — so every following
/// directoryCreate/fileWrite failed with a permission error (GH #380). That
/// platform difference is exactly why it went unseen in local development.
///
/// Lives here rather than in `cfml-stdlib` because the VM's sandbox intercept
/// needs it too, and `cfml-stdlib` is an OPTIONAL dependency of `cfml-vm`
/// (feature `s3`) — calling into it unconditionally builds only when that
/// feature happens to be on.
pub fn temp_dir_with_separator() -> String {
    let mut dir = std::env::temp_dir().to_string_lossy().to_string();
    if !dir.ends_with(std::path::MAIN_SEPARATOR) && !dir.ends_with('/') {
        dir.push(std::path::MAIN_SEPARATOR);
    }
    dir
}
