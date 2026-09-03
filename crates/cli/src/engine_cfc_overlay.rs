//! Engine-bundled compat CFCs, overlaid onto the serve-mode VFS so reserved
//! component names resolve out of the box:
//!   - the socket.io-lucee compat trio (`new SocketIoServer()` etc.)
//!   - Lucee's built-in `new Query()` builder + its `Result` return object
//!   - Lucee's built-in `new Mail()` message builder (GH #356)
//!
//! [`EngineCfcOverlay`] wraps the real filesystem and serves an engine copy
//! for any path whose basename matches a reserved name — *only when the base
//! VFS does not already have that file*. So a user's own same-named file always
//! wins; the engine copy is a fallback. All other paths pass straight through.

use std::io;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cfml_common::vfs::{Vfs, VfsDirEntry};

const SERVER_CFC: &str = include_str!("../assets/socketio/SocketIoServer.cfc");
const NAMESPACE_CFC: &str = include_str!("../assets/socketio/SocketIoNamespace.cfc");
const SOCKET_CFC: &str = include_str!("../assets/socketio/SocketIoSocket.cfc");
const QUERY_CFC: &str = include_str!("../assets/lucee/Query.cfc");
const RESULT_CFC: &str = include_str!("../assets/lucee/Result.cfc");
const MAIL_CFC: &str = include_str!("../assets/lucee/Mail.cfc");

/// The embedded source for a reserved compat-CFC path, keyed by basename
/// (case-insensitive). `None` for any other path.
/// Allocation-free: this now runs first on EVERY overlay call (see `overlaid`),
/// so it splits the basename in place and compares case-insensitively rather
/// than building a normalised lowercase copy of the path (GH #299).
fn engine_cfc(path: &str) -> Option<&'static str> {
    let base = match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    };
    // `eq_ignore_ascii_case` length-checks first, so a non-matching basename
    // costs a handful of length comparisons.
    for (name, src) in [
        ("SocketIoServer.cfc", SERVER_CFC),
        ("SocketIoNamespace.cfc", NAMESPACE_CFC),
        ("SocketIoSocket.cfc", SOCKET_CFC),
        ("Query.cfc", QUERY_CFC),
        ("Result.cfc", RESULT_CFC),
        ("Mail.cfc", MAIL_CFC),
    ] {
        if base.eq_ignore_ascii_case(name) {
            return Some(src);
        }
    }
    None
}

pub struct EngineCfcOverlay {
    base: Arc<dyn Vfs>,
}

impl std::fmt::Debug for EngineCfcOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineCfcOverlay").finish()
    }
}

impl EngineCfcOverlay {
    pub fn new(base: Arc<dyn Vfs>) -> Self {
        Self { base }
    }

    /// Whether this path could be an engine CFC at all: reserved basename AND
    /// the directory it sits in genuinely exists.
    ///
    /// The directory test is what keeps the overlay from answering for a
    /// PACKAGED name. Matching on the basename alone meant `new zzzz.Query()`
    /// resolved to `<root>/zzzz/Query.cfc` and was served the engine's Query
    /// builder even though no `zzzz` package exists — silently handing back a
    /// working object where Lucee throws "could not find component [zzzz.Query]".
    /// Same for `Mail`, `Result` and the socket.io trio.
    ///
    /// This does not make the overlay fully package-aware: a path under a
    /// directory that DOES exist but has no such file (`realdir.Query`) is still
    /// answered. Closing that needs the unqualified/qualified distinction from
    /// the resolver, which does not reach the VFS — the name arrives here already
    /// flattened to a path.
    ///
    /// The reserved-name test comes FIRST because it is pure string work: for the
    /// ~100% of paths that are ordinary application files it answers `None`
    /// without touching the filesystem, so only a reserved basename ever pays the
    /// directory stat.
    fn engine_candidate(&self, path: &str) -> Option<&'static str> {
        let src = engine_cfc(path)?;
        let parent = match path.rfind(['/', '\\']) {
            Some(i) => &path[..i],
            None => "",
        };
        if !parent.is_empty() && !self.base.is_dir(parent) {
            return None;
        }
        Some(src)
    }

    /// Whether this path should be served from the overlay (an engine candidate
    /// AND the base FS doesn't have a real file there — real files always win).
    ///
    /// Probing `base.exists()` before the name test made every overlay call a
    /// double-stat — 3.8% of production CPU (GH #299) — so it stays last.
    fn overlaid(&self, path: &str) -> Option<&'static str> {
        let src = self.engine_candidate(path)?;
        if self.base.exists(path) {
            return None;
        }
        Some(src)
    }
}

impl Vfs for EngineCfcOverlay {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        match self.overlaid(path) {
            Some(src) => Ok(src.to_string()),
            None => self.base.read_to_string(path),
        }
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        match self.overlaid(path) {
            Some(src) => Ok(src.as_bytes().to_vec()),
            None => self.base.read(path),
        }
    }

    /// Forwarded so a `loop file=` on an ordinary path still streams; only the
    /// handful of overlaid engine CFCs (already in-memory strings) go eager.
    /// `opts.charset` is not applied to those: they are Rust `&str` already
    /// decoded, not bytes on disk.
    fn open_chunks(
        &self,
        path: &str,
        opts: cfml_common::vfs::FileCursorOpts,
    ) -> io::Result<Box<dyn cfml_common::vfs::VfsFileChunks>> {
        match self.overlaid(path) {
            Some(src) => Ok(Box::new(cfml_common::vfs::EagerChunks::new(src, opts.chunking))),
            None => self.base.open_chunks(path, opts),
        }
    }

    fn exists(&self, path: &str) -> bool {
        // Ordinary paths: one stat, straight through (see `overlaid`).
        if self.engine_candidate(path).is_none() {
            return self.base.exists(path);
        }
        // A reserved name always exists — the overlay backs it whether or not the
        // base FS has a real file there.
        true
    }

    fn is_file(&self, path: &str) -> bool {
        // Ordinary paths: one stat, straight through. Only a reserved *name* can
        // reach the second (`exists`) probe — previously every miss paid it.
        if self.engine_candidate(path).is_none() {
            return self.base.is_file(path);
        }
        self.base.is_file(path) || !self.base.exists(path)
    }

    fn is_dir(&self, path: &str) -> bool {
        self.base.is_dir(path)
    }

    fn read_dir(&self, path: &str) -> io::Result<Vec<VfsDirEntry>> {
        self.base.read_dir(path)
    }

    fn modified(&self, path: &str) -> io::Result<SystemTime> {
        match self.overlaid(path) {
            // Stable mtime so the bytecode cache treats the engine CFCs as fixed.
            Some(_) => Ok(UNIX_EPOCH),
            None => self.base.modified(path),
        }
    }

    fn canonicalize(&self, path: &str) -> io::Result<String> {
        match self.base.canonicalize(path) {
            Ok(p) => Ok(p),
            // A reserved-name path that isn't on disk canonicalizes to itself so
            // the VM can use it as a stable source_file key.
            Err(e) => {
                if engine_cfc(path).is_some() {
                    Ok(path.to_string())
                } else {
                    Err(e)
                }
            }
        }
    }
}
