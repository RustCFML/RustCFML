//! `java.io.FileWriter` / `java.io.BufferedWriter` and `com.opencsv.CSVWriter`.
//!
//! Preside's `CsvWriter.cfc` — the engine behind every admin data export and
//! every form-builder submission download — is built on exactly this pair:
//!
//! ```cfml
//! fileWriter    = CreateObject( "java", "java.io.FileWriter" ).init( filePath );
//! openCsvWriter = CreateObject( "java", "com.opencsv.CSVWriter", jars )
//!                     .init( fileWriter, JavaCast( "char", delimiter ) );
//! openCsvWriter.writeNext( [ "a", "b" ] );
//! openCsvWriter.flush();
//! openCsvWriter.close();
//! ```
//!
//! The record encoding is the `csvFormatRow()` builtin, which follows RFC 4180 and
//! reproduces opencsv's defaults (quote every field, double an embedded quote,
//! `\n` terminator). This module is the file-handle half.
//!
//! **Buffering.** An export can be hundreds of thousands of rows, so rows are
//! accumulated and appended to the file whenever the pending buffer passes
//! [`FLUSH_THRESHOLD`], as well as on explicit `flush()`/`close()`. That bounds
//! memory without turning every `writeNext()` into a syscall. Preside's CFC does
//! call `close()`; a writer that is dropped without one loses only the tail that
//! never reached the threshold, which is the same hazard the JVM has and the
//! reason the CFC closes in a `finally`.
//!
//! I/O goes through `std::fs`, matching the `java.io.FileOutputStream` /
//! `FileInputStream` shims next door rather than routing through the file BIFs —
//! these classes *are* the file layer, and the VM applies the same
//! existence-cache invalidation to them as to any other mutating shim.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};
use std::io::Write;

pub const FILE_WRITER_CLASS: &str = "java.io.filewriter";
pub const BUFFERED_WRITER_CLASS: &str = "java.io.bufferedwriter";
pub const PRINT_WRITER_CLASS: &str = "java.io.printwriter";
pub const CSV_WRITER_CLASS: &str = "com.opencsv.csvwriter";

/// Append to disk once the pending text passes this many bytes.
const FLUSH_THRESHOLD: usize = 64 * 1024;

pub fn is_writer_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        FILE_WRITER_CLASS | BUFFERED_WRITER_CLASS | PRINT_WRITER_CLASS | CSV_WRITER_CLASS
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(CfmlValue::strukt(shim(class_lower)))
}

fn field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn field_str(object: &CfmlValue, key: &str) -> String {
    field(object, key).map(|v| v.as_string()).unwrap_or_default()
}

fn set(object: &CfmlValue, key: &str, value: CfmlValue) {
    if let CfmlValue::Struct(s) = object {
        s.insert(key.to_string(), value);
    }
}

fn io_error(op: &str, path: &str, e: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("java.io.IOException: cannot {} '{}': {}", op, path, e),
        CfmlErrorType::Custom("java.io.IOException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!("{}.{}() is not supported by RustCFML's writer shims", class, method),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// Follow a writer chain down to the underlying file path. A `CSVWriter` wraps a
/// `BufferedWriter`/`PrintWriter` wraps a `FileWriter`; every level stores either
/// its own `__path` or the writer it delegates to under `__sink`.
fn resolve_path(object: &CfmlValue) -> String {
    let mut cur = object.clone();
    for _ in 0..8 {
        let direct = field_str(&cur, "__path");
        if !direct.is_empty() {
            return direct;
        }
        match field(&cur, "__sink") {
            Some(next @ CfmlValue::Struct(_)) => cur = next,
            _ => break,
        }
    }
    String::new()
}

/// Append `text` to `path`, creating the file if needed.
fn append(path: &str, text: &str) -> Result<(), CfmlError> {
    if text.is_empty() {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| io_error("open for writing", path, e))?;
    f.write_all(text.as_bytes())
        .map_err(|e| io_error("write to", path, e))
}

/// Push the receiver's pending text to disk and clear it.
fn flush_pending(object: &CfmlValue) -> Result<(), CfmlError> {
    let pending = field_str(object, "__pending");
    if pending.is_empty() {
        return Ok(());
    }
    let path = resolve_path(object);
    if path.is_empty() {
        return Err(CfmlError::new(
            "java.io.IOException: writer has no target file — construct it with \
             init( path ) or init( fileWriter )"
                .to_string(),
            CfmlErrorType::Custom("java.io.IOException".to_string()),
        ));
    }
    append(&path, &pending)?;
    set(object, "__pending", CfmlValue::string(String::new()));
    Ok(())
}

/// Buffer `text` on the receiver, spilling to disk past the threshold.
fn buffer(object: &CfmlValue, text: &str) -> Result<(), CfmlError> {
    let mut pending = field_str(object, "__pending");
    pending.push_str(text);
    let over = pending.len() >= FLUSH_THRESHOLD;
    set(object, "__pending", CfmlValue::string(pending));
    if over {
        flush_pending(object)?;
    }
    Ok(())
}

// ── java.io.FileWriter / BufferedWriter / PrintWriter ────────────────────────

pub fn handle_writer(
    class: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut m = shim(class);
            m.insert("__pending".to_string(), CfmlValue::string(String::new()));

            match args.first() {
                // FileWriter( String path ) / FileWriter( File f )
                Some(CfmlValue::Struct(s)) if s.contains_key("__java_shim") => {
                    if let Some(p) = s.get("__file_path") {
                        // a java.io.File
                        let path = p.as_string();
                        let append_mode = matches!(args.get(1), Some(CfmlValue::Bool(true)));
                        open_target(&path, append_mode)?;
                        m.insert("__path".to_string(), CfmlValue::string(path));
                    } else {
                        // BufferedWriter( Writer ) / CSVWriter-style delegation
                        m.insert("__sink".to_string(), args[0].clone());
                    }
                }
                Some(v) => {
                    let path = v.as_string();
                    let append_mode = matches!(args.get(1), Some(CfmlValue::Bool(true)));
                    open_target(&path, append_mode)?;
                    m.insert("__path".to_string(), CfmlValue::string(path));
                }
                // `createObject(...)` with no `.init()` yet — the path arrives on
                // the explicit init call, exactly as FileOutputStream handles it.
                None => {}
            }
            Ok(CfmlValue::strukt(m))
        }
        "write" | "append" | "print" => {
            let text = args.first().map(|v| v.as_string()).unwrap_or_default();
            buffer(object, &text)?;
            // append() is fluent in Java; write()/print() are void. Returning the
            // receiver for all three is harmless (a void result is discarded).
            Ok(object.clone())
        }
        "println" => {
            let text = args.first().map(|v| v.as_string()).unwrap_or_default();
            buffer(object, &format!("{}\n", text))?;
            Ok(CfmlValue::Null)
        }
        "newline" => {
            buffer(object, "\n")?;
            Ok(CfmlValue::Null)
        }
        "flush" => {
            flush_pending(object)?;
            // A wrapped writer must flush its sink too, or the bytes stop one
            // level short of the file.
            if let Some(sink @ CfmlValue::Struct(_)) = field(object, "__sink") {
                flush_pending(&sink)?;
            }
            Ok(CfmlValue::Null)
        }
        "close" => {
            flush_pending(object)?;
            if let Some(sink @ CfmlValue::Struct(_)) = field(object, "__sink") {
                flush_pending(&sink)?;
                set(&sink, "__closed", CfmlValue::Bool(true));
            }
            set(object, "__closed", CfmlValue::Bool(true));
            Ok(CfmlValue::Null)
        }
        other => Err(unsupported(class, other)),
    }
}

/// Create or truncate the target, so constructing the writer has the same
/// observable effect it does on the JVM even if nothing is ever written.
fn open_target(path: &str, append_mode: bool) -> Result<(), CfmlError> {
    if path.is_empty() {
        return Ok(());
    }
    if append_mode {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(|_| ())
            .map_err(|e| io_error("open for appending", path, e))
    } else {
        std::fs::write(path, b"").map_err(|e| io_error("open for writing", path, e))
    }
}

// ── com.opencsv.CSVWriter ────────────────────────────────────────────────────

/// `format_row` is the `csvFormatRow()` builtin.
pub fn handle_csv_writer(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    format_row: impl Fn(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    // `&dyn` rather than a generic: `writeAll` calls back into the row path per
    // row, and a generic closure parameter makes that a recursive instantiation
    // the compiler cannot monomorphise.
    csv_dispatch(method, args, object, &format_row)
}

fn csv_dispatch(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    format_row: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match method {
        // CSVWriter( Writer w ) and the (w, sep), (w, sep, quote),
        // (w, sep, quote, escape) and (w, sep, quote, escape, lineEnd) overloads.
        "init" => {
            let mut m = shim(CSV_WRITER_CLASS);
            m.insert("__pending".to_string(), CfmlValue::string(String::new()));
            match args.first() {
                Some(w @ CfmlValue::Struct(_)) => {
                    m.insert("__sink".to_string(), w.clone());
                }
                Some(v) => {
                    // Some callers hand a path straight to CSVWriter.
                    let path = v.as_string();
                    open_target(&path, false)?;
                    m.insert("__path".to_string(), CfmlValue::string(path));
                }
                None => {}
            }
            let ch = |i: usize, default: &str| -> String {
                match args.get(i) {
                    Some(CfmlValue::Null) | None => default.to_string(),
                    Some(v) => {
                        let s = v.as_string();
                        if s.is_empty() { default.to_string() } else { s.chars().next().unwrap().to_string() }
                    }
                }
            };
            m.insert("__delimiter".to_string(), CfmlValue::string(ch(1, ",")));
            m.insert("__quote".to_string(), CfmlValue::string(ch(2, "\"")));
            m.insert(
                "__escape".to_string(),
                CfmlValue::string(ch(3, &ch(2, "\""))),
            );
            m.insert(
                "__lineend".to_string(),
                CfmlValue::string(match args.get(4) {
                    Some(CfmlValue::Null) | None => "\n".to_string(),
                    Some(v) => v.as_string(),
                }),
            );
            Ok(CfmlValue::strukt(m))
        }
        // writeNext( String[] ) — opencsv quotes every field. writeNext( line,
        // applyQuotesToAll ) is the two-arg overload.
        "writenext" => {
            let values = args.first().cloned().unwrap_or_else(|| CfmlValue::array(Vec::new()));
            let quote_all = match args.get(1) {
                Some(CfmlValue::Bool(b)) => *b,
                _ => true,
            };
            let line = format_row(vec![
                values,
                CfmlValue::string(field_str(object, "__delimiter")),
                CfmlValue::string(field_str(object, "__quote")),
                CfmlValue::string(field_str(object, "__escape")),
                CfmlValue::Bool(quote_all),
            ])?
            .as_string();
            let mut lineend = field_str(object, "__lineend");
            if lineend.is_empty() {
                lineend = "\n".to_string();
            }
            buffer(object, &format!("{}{}", line, lineend))?;
            Ok(CfmlValue::Null)
        }
        // writeAll( List<String[]> ) — one call per row, same encoding.
        "writeall" => {
            let rows = match args.first() {
                Some(CfmlValue::Array(a)) => a.snapshot(),
                _ => Vec::new(),
            };
            for row in rows {
                csv_dispatch("writenext", vec![row], object, format_row)?;
            }
            Ok(CfmlValue::Null)
        }
        "flush" | "close" => handle_writer(CSV_WRITER_CLASS, method, args, object),
        // opencsv reports whether any write hit an IOException. We surface I/O
        // failures as thrown java.io.IOExceptions at the point they happen, so by
        // the time a caller could ask, there is nothing pending to report.
        "checkerror" => Ok(CfmlValue::Bool(false)),
        other => Err(unsupported("com.opencsv.CSVWriter", other)),
    }
}
