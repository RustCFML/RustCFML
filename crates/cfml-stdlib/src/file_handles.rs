//! Real file handles for `fileOpen()` and friends.
//!
//! Before this module the handle was a struct holding the path and a line
//! counter, and every `fileReadLine()` / `fileIsEof()` re-read the WHOLE file
//! (`fileIsEof` also counted its lines) — quadratic on any line loop, and
//! 447 µs per `fileIsEof` on a 2 KB file. Preside's boot opens every view file
//! to read its first line for a `@feature` marker; on an 857-view site that
//! one scan cost over a second of a 4.7 s boot.
//!
//! A handle is now an entry in a process-wide registry keyed by an id the
//! returned struct carries (`__handle_id`). Read modes hold a `BufReader`,
//! write/append modes a `BufWriter`; `fileClose` drops the entry (flushing).
//! Write modes create (or truncate) the file at `fileOpen` time, as Lucee
//! does (docs/known-issues.md #49). The struct keeps `path` plus Lucee's
//! `filename`/`filepath`/`mode`/`status` keys for code that inspects it.
//!
//! The registry is global rather than per-VM on purpose: the stdlib has no VM
//! handle, and a leaked handle (script that never closes) is bounded by the
//! script's own behaviour exactly as it is on Lucee.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub const HANDLE_ID_KEY: &str = "__handle_id";

enum Io {
    Read(BufReader<File>),
    Write(BufWriter<File>),
}

struct OpenFile {
    io: Io,
}

static HANDLES: Lazy<Mutex<HashMap<u64, OpenFile>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn arg_str(args: &[CfmlValue], i: usize) -> String {
    args.get(i).map(|v| v.as_string()).unwrap_or_default()
}

/// The registry id carried by a handle struct, if `v` is one.
pub fn handle_id(v: Option<&CfmlValue>) -> Option<u64> {
    match v {
        Some(CfmlValue::Struct(s)) => match s.get(HANDLE_ID_KEY) {
            Some(CfmlValue::Int(id)) if id > 0 => Some(id as u64),
            _ => None,
        },
        _ => None,
    }
}

fn with_handle<R>(
    fn_name: &str,
    args: &[CfmlValue],
    f: impl FnOnce(&mut OpenFile) -> Result<R, CfmlError>,
) -> Result<R, CfmlError> {
    let id = handle_id(args.first())
        .ok_or_else(|| CfmlError::runtime(format!("{}() requires an open file handle", fn_name)))?;
    let mut reg = HANDLES.lock().unwrap_or_else(|e| e.into_inner());
    let entry = reg
        .get_mut(&id)
        .ok_or_else(|| CfmlError::runtime(format!("{}(): the file handle is closed", fn_name)))?;
    f(entry)
}

fn io_err(fn_name: &str, e: std::io::Error) -> CfmlError {
    CfmlError::runtime(format!("{}(): {}", fn_name, e))
}

/// `fileOpen( path [, mode="read" [, charset]] )`.
pub fn open(args: Vec<CfmlValue>) -> CfmlResult {
    let path = arg_str(&args, 0);
    let mode = if args.len() > 1 { arg_str(&args, 1).to_ascii_lowercase() } else { "read".to_string() };
    let io = match mode.as_str() {
        "read" | "readbinary" => match File::open(&path) {
            Ok(f) => Io::Read(BufReader::new(f)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CfmlError::file_not_found(format!("The file [{}] does not exist", path)));
            }
            Err(e) => return Err(io_err("fileOpen", e)),
        },
        // Lucee creates the file at open time in the write modes (known-issues #49).
        "write" | "writebinary" => Io::Write(BufWriter::new(
            File::create(&path).map_err(|e| io_err("fileOpen", e))?,
        )),
        "append" | "appendbinary" => Io::Write(BufWriter::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| io_err("fileOpen", e))?,
        )),
        other => {
            return Err(CfmlError::runtime(format!(
                "fileOpen(): invalid mode [{}]; expected read, readBinary, write, writeBinary or append",
                other
            )))
        }
    };
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    HANDLES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, OpenFile { io });

    let filename = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut handle = ValueMap::default();
    handle.insert(HANDLE_ID_KEY.to_string(), CfmlValue::Int(id as i64));
    handle.insert("path".to_string(), CfmlValue::string(path.clone()));
    handle.insert("filepath".to_string(), CfmlValue::string(path));
    handle.insert("filename".to_string(), CfmlValue::string(filename));
    handle.insert("mode".to_string(), CfmlValue::string(mode));
    handle.insert("status".to_string(), CfmlValue::string("open".to_string()));
    handle.insert("isOpen".to_string(), CfmlValue::Bool(true));
    Ok(CfmlValue::strukt(handle))
}

/// `fileClose( handle )` — drops the entry, flushing a writer. Closing an
/// already-closed or non-handle value is a no-op (as before).
pub fn close(args: Vec<CfmlValue>) -> CfmlResult {
    if let Some(id) = handle_id(args.first()) {
        let removed = HANDLES.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(OpenFile { io: Io::Write(mut w) }) = removed {
            w.flush().map_err(|e| io_err("fileClose", e))?;
        }
    }
    Ok(CfmlValue::Null)
}

/// `fileReadLine( handle )` — the next line without its terminator; an empty
/// string at end of file.
pub fn read_line(args: Vec<CfmlValue>) -> CfmlResult {
    with_handle("fileReadLine", &args, |h| match &mut h.io {
        Io::Read(r) => {
            let mut buf = Vec::new();
            r.read_until(b'\n', &mut buf).map_err(|e| io_err("fileReadLine", e))?;
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            Ok(CfmlValue::string(String::from_utf8_lossy(&buf).into_owned()))
        }
        Io::Write(_) => Err(CfmlError::runtime(
            "fileReadLine(): the file was opened for writing".to_string(),
        )),
    })
}

/// `fileIsEof( handle )`.
pub fn is_eof(args: Vec<CfmlValue>) -> CfmlResult {
    if handle_id(args.first()).is_none() {
        return Ok(CfmlValue::Bool(true));
    }
    with_handle("fileIsEof", &args, |h| match &mut h.io {
        Io::Read(r) => Ok(CfmlValue::Bool(
            r.fill_buf().map_err(|e| io_err("fileIsEof", e))?.is_empty(),
        )),
        Io::Write(_) => Ok(CfmlValue::Bool(true)),
    })
}

/// `fileRead( handle [, charsOrBytes] )` — the rest of the file, or the next
/// `n` bytes, as a string.
pub fn read(args: Vec<CfmlValue>) -> CfmlResult {
    let n = args.get(1).and_then(|v| match v {
        CfmlValue::Int(i) if *i > 0 => Some(*i as u64),
        CfmlValue::Double(d) if *d > 0.0 => Some(*d as u64),
        CfmlValue::String(s) => s.parse::<u64>().ok().filter(|n| *n > 0),
        _ => None,
    });
    let bytes = read_bytes("fileRead", &args, n)?;
    Ok(CfmlValue::string(String::from_utf8_lossy(&bytes).into_owned()))
}

/// `fileReadBinary( handle [, bytes] )`.
pub fn read_binary(args: Vec<CfmlValue>) -> CfmlResult {
    let n = args.get(1).and_then(|v| match v {
        CfmlValue::Int(i) if *i > 0 => Some(*i as u64),
        CfmlValue::Double(d) if *d > 0.0 => Some(*d as u64),
        _ => None,
    });
    Ok(CfmlValue::Binary(read_bytes("fileReadBinary", &args, n)?))
}

fn read_bytes(fn_name: &str, args: &[CfmlValue], n: Option<u64>) -> Result<Vec<u8>, CfmlError> {
    with_handle(fn_name, args, |h| match &mut h.io {
        Io::Read(r) => {
            let mut out = Vec::new();
            match n {
                Some(n) => r.take(n).read_to_end(&mut out),
                None => r.read_to_end(&mut out),
            }
            .map_err(|e| io_err(fn_name, e))?;
            Ok(out)
        }
        Io::Write(_) => Err(CfmlError::runtime(format!(
            "{}(): the file was opened for writing",
            fn_name
        ))),
    })
}

/// `fileWrite( handle, data )` / `fileWriteLine( handle, data )`.
pub fn write(fn_name: &'static str, args: Vec<CfmlValue>, newline: bool) -> CfmlResult {
    with_handle(fn_name, &args, |h| match &mut h.io {
        Io::Write(w) => {
            match args.get(1) {
                Some(CfmlValue::Binary(bytes)) => w.write_all(bytes),
                other => w.write_all(other.map(|v| v.as_string()).unwrap_or_default().as_bytes()),
            }
            .map_err(|e| io_err(fn_name, e))?;
            if newline {
                w.write_all(b"\n").map_err(|e| io_err(fn_name, e))?;
            }
            Ok(CfmlValue::Null)
        }
        Io::Read(_) => Err(CfmlError::runtime(format!(
            "{}(): the file was opened for reading",
            fn_name
        ))),
    })
}

/// `fileSeek( handle, position )` — absolute byte offset.
pub fn seek(args: Vec<CfmlValue>) -> CfmlResult {
    let pos = args.get(1).map(|v| v.as_string().parse::<u64>().unwrap_or(0)).unwrap_or(0);
    with_handle("fileSeek", &args, |h| {
        match &mut h.io {
            Io::Read(r) => r.seek(SeekFrom::Start(pos)).map(|_| ()),
            Io::Write(w) => w.seek(SeekFrom::Start(pos)).map(|_| ()),
        }
        .map_err(|e| io_err("fileSeek", e))?;
        Ok(CfmlValue::Null)
    })
}

/// `fileSkipBytes( handle, count )` — relative to the current position.
pub fn skip_bytes(args: Vec<CfmlValue>) -> CfmlResult {
    let n = args.get(1).map(|v| v.as_string().parse::<i64>().unwrap_or(0)).unwrap_or(0);
    with_handle("fileSkipBytes", &args, |h| {
        match &mut h.io {
            Io::Read(r) => r.seek(SeekFrom::Current(n)).map(|_| ()),
            Io::Write(w) => w.seek(SeekFrom::Current(n)).map(|_| ()),
        }
        .map_err(|e| io_err("fileSkipBytes", e))?;
        Ok(CfmlValue::Null)
    })
}
