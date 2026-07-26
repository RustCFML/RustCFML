//! CFML log appenders — the file sink behind `<cflog>` / `writeLog()`.
//!
//! Lucee routes `<cflog file="myapp">` through log4j2 into
//! `<log-dir>/myapp.log` with a buffered rolling-file appender. This module is
//! the RustCFML equivalent: a process-wide registry of named appenders, each
//! holding a cached `BufWriter<File>`, so a firing log line is an in-memory
//! append rather than an unbuffered `write(2)` under the global stderr lock.
//!
//! The line format is byte-compatible with Lucee 7's default log layout
//! (verified against Lucee 7.0.4), so existing log tooling keeps working:
//!
//! ```text
//! "Severity","ThreadID","Date","Time","Context","Application","Message"
//! "ERROR","main","07/26/2026","22:56:48","http://127.0.0.1:8500","MyApp","boom"
//! ```
//!
//! The header row is written once, when the file is created.
//!
//! **Buffering and durability.** Each line is flushed as it is written, which
//! is log4j2's default (`immediateFlush`) and what makes `tail -f` on a log file
//! work. The win over the old code is the *cached handle* — no open/close per
//! line, and no global stderr lock. Setting `flushEachLine: false` batches lines
//! in the `BufWriter` instead, flushed by [`flush_all`] at request end (serve
//! mode) and process exit: cheaper for a chatty logger, but a line is not
//! visible until its request completes.
//!
//! **wasm.** `wasm32` targets have no filesystem, so `write_entry` there routes
//! the formatted line through the `log` crate instead of a file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use parking_lot::Mutex;

// ─────────────────────────────────────────────
// Levels
// ─────────────────────────────────────────────

/// Log severities, ordered least→most severe. The names match log4j2's (which
/// is what Lucee writes into the `Severity` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    /// The `Severity` column value.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Fatal => "FATAL",
        }
    }
}

/// Parse a CFML `type=` attribute value. Accepts exactly the spellings Lucee 7
/// accepts — anything else is an error, matching Lucee's
/// "Invalid value for attribute type [x]".
pub fn parse_type_attr(s: &str) -> Option<LogLevel> {
    match s.trim().to_ascii_lowercase().as_str() {
        "information" | "info" => Some(LogLevel::Info),
        "warning" | "warn" => Some(LogLevel::Warn),
        "error" => Some(LogLevel::Error),
        "fatal" => Some(LogLevel::Fatal),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

/// Parse a configured level threshold. `off`/`none` yields `Ok(None)` — the
/// logger is silenced. An unrecognised name yields `Err`.
pub fn parse_level_threshold(s: &str) -> Result<Option<LogLevel>, ()> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err(());
    }
    if lower == "off" || lower == "none" {
        return Ok(None);
    }
    parse_type_attr(&lower).map(Some).ok_or(())
}

// ─────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────

/// Lucee's log4j2 rolling-appender defaults.
const DEFAULT_MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
const DEFAULT_MAX_FILES: u32 = 10;

/// Process-wide appender configuration, seeded once at startup from
/// `.cfconfig.json`'s `logging` block (see `cfml_config::LoggingCfg`).
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Directory that log files live in. `None` disables file logging entirely
    /// (lines fall back to stderr so nothing is silently dropped).
    pub directory: Option<PathBuf>,
    /// Threshold applied to loggers with no explicit entry in `levels`.
    /// Defaults to `Trace` — i.e. log everything, which is what Lucee does for
    /// an ad-hoc `file=` logger it has no configuration for.
    pub default_level: Option<LogLevel>,
    /// Per-logger thresholds, keyed by lower-cased log name. `None` = silenced.
    pub levels: HashMap<String, Option<LogLevel>>,
    /// Rotate once a file would exceed this many bytes.
    pub max_file_size: u64,
    /// How many rotated generations (`name.log.1.bak` … `name.log.N.bak`) to keep.
    pub max_files: u32,
    /// Also echo every line to stderr (the pre-v0.528 behaviour). Off by
    /// default — Lucee does not echo to the console either.
    pub echo_stderr: bool,
    /// Flush after every line (log4j2's `immediateFlush`, and its default).
    /// `true` is what makes `tail -f` on a log file work. Set `false` to batch
    /// lines in the `BufWriter` until request end — cheaper for a chatty logger,
    /// at the cost of a line only becoming visible once the request finishes.
    pub flush_each_line: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            directory: None,
            default_level: Some(LogLevel::Trace),
            levels: HashMap::new(),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_files: DEFAULT_MAX_FILES,
            echo_stderr: false,
            flush_each_line: true,
        }
    }
}

fn config() -> &'static Mutex<Option<LoggingConfig>> {
    static CONFIG: OnceLock<Mutex<Option<LoggingConfig>>> = OnceLock::new();
    CONFIG.get_or_init(|| Mutex::new(None))
}

/// Install the process-wide logging configuration. Called by the CLI once the
/// webroot and `.cfconfig.json` are known. Replaces any previous config and
/// flushes + drops open appenders so a changed directory takes effect.
pub fn configure(cfg: LoggingConfig) {
    #[cfg(not(target_arch = "wasm32"))]
    native::close_all();
    *config().lock() = Some(cfg);
}

/// The configured log directory, if file logging is active.
pub fn logs_directory() -> Option<PathBuf> {
    config().lock().as_ref().and_then(|c| c.directory.clone())
}

/// Whether a line at `level` for log `name` would be written. Callers can use
/// this to skip formatting work for a suppressed line.
pub fn is_enabled(name: &str, level: LogLevel) -> bool {
    let guard = config().lock();
    let cfg = match guard.as_ref() {
        Some(c) => c,
        // Unconfigured: fall back to stderr, and stderr takes everything.
        None => return true,
    };
    threshold(cfg, name).is_some_and(|min| level >= min)
}

fn threshold(cfg: &LoggingConfig, name: &str) -> Option<LogLevel> {
    match cfg.levels.get(&name.to_ascii_lowercase()) {
        Some(explicit) => *explicit,
        None => cfg.default_level,
    }
}

// ─────────────────────────────────────────────
// Writing
// ─────────────────────────────────────────────

/// Reject the file names Lucee rejects. A log name becomes a bare filename in
/// the log directory, so a path separator would let it escape — Lucee raises an
/// `application` error, and so do we, with the same message.
pub fn validate_log_name(name: &str) -> Result<(), String> {
    if name.contains('/') || name.contains('\\') {
        return Err(format!(
            "Invalid value [{}] for the attribute [file] for tag [log], \
             it must be a valid filename, file separators like [\\/] are not allowed",
            name
        ));
    }
    Ok(())
}

/// Append one entry to the named log.
///
/// `name` is the `file=`/`log=` value with no extension (`.log` is appended).
/// `context` is the request's base URL (empty in CLI mode) and `application`
/// the application name — both are Lucee columns. Returns `Err` only for a
/// caller mistake (an invalid log name); I/O failures fall back to stderr so a
/// log line is never silently lost.
pub fn write_entry(
    name: &str,
    level: LogLevel,
    context: &str,
    application: &str,
    message: &str,
) -> Result<(), String> {
    validate_log_name(name)?;

    let (directory, echo_stderr, flush_each_line, max_file_size, max_files) = {
        let guard = config().lock();
        match guard.as_ref() {
            Some(cfg) => {
                if !threshold(cfg, name).is_some_and(|min| level >= min) {
                    return Ok(());
                }
                (
                    cfg.directory.clone(),
                    cfg.echo_stderr,
                    cfg.flush_each_line,
                    cfg.max_file_size,
                    cfg.max_files,
                )
            }
            None => (None, true, false, DEFAULT_MAX_FILE_SIZE, DEFAULT_MAX_FILES),
        }
    };

    let line = format_line(level, context, application, message);

    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = directory {
            match native::append(&dir, name, &line, max_file_size, max_files, flush_each_line) {
                Ok(()) => {
                    if echo_stderr {
                        eprint!("{}", line);
                    }
                    return Ok(());
                }
                Err(e) => {
                    // Surface the failure rather than dropping the line.
                    eprint!("[cflog: {} — falling back to stderr] {}", e, line);
                    return Ok(());
                }
            }
        }
        eprint!("{}", line);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (directory, echo_stderr, flush_each_line, max_file_size, max_files);
        // No filesystem on wasm32 (Cloudflare Workers et al) — the platform log
        // sink is the only destination.
        log::info!("{}", line.trim_end());
    }
    Ok(())
}

/// Format one Lucee-layout CSV record, newline-terminated.
fn format_line(level: LogLevel, context: &str, application: &str, message: &str) -> String {
    let (date, time) = local_date_time();
    let mut out = String::with_capacity(message.len() + 96);
    csv_field(&mut out, level.as_str());
    out.push(',');
    csv_field(&mut out, &thread_label());
    out.push(',');
    csv_field(&mut out, &date);
    out.push(',');
    csv_field(&mut out, &time);
    out.push(',');
    csv_field(&mut out, context);
    out.push(',');
    csv_field(&mut out, application);
    out.push(',');
    csv_field(&mut out, message);
    out.push('\n');
    out
}

/// The `Severity,...` header row, written when a log file is created.
pub const HEADER: &str =
    "\"Severity\",\"ThreadID\",\"Date\",\"Time\",\"Context\",\"Application\",\"Message\"\n";

/// CSV-quote one field. Lucee doubles embedded quotes and leaves newlines raw
/// (a multi-line message spans lines in the file); we match that exactly.
fn csv_field(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
}

/// `("MM/DD/YYYY", "HH:MM:SS")` in server-local time, as Lucee writes them.
fn local_date_time() -> (String, String) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = chrono::Local::now();
        (
            now.format("%m/%d/%Y").to_string(),
            now.format("%H:%M:%S").to_string(),
        )
    }
    #[cfg(target_arch = "wasm32")]
    {
        // No local-timezone database on wasm32; UTC from the JS clock.
        let ms = crate::clock::now_unix_millis() as i64;
        match chrono::DateTime::from_timestamp_millis(ms) {
            Some(dt) => (
                dt.format("%m/%d/%Y").to_string(),
                dt.format("%H:%M:%S").to_string(),
            ),
            None => (String::new(), String::new()),
        }
    }
}

/// Lucee writes the servlet worker thread name here. The Rust equivalent is the
/// thread's name when it has one, else its opaque id.
fn thread_label() -> String {
    let current = std::thread::current();
    match current.name() {
        Some(n) => n.to_string(),
        None => {
            let id = format!("{:?}", current.id());
            let digits = id.trim_start_matches("ThreadId(").trim_end_matches(')');
            format!("thread-{}", digits)
        }
    }
}

/// Flush every open appender. Called at request end (serve mode) and process
/// exit; cheap when there is nothing buffered.
pub fn flush_all() {
    #[cfg(not(target_arch = "wasm32"))]
    native::flush_all();
}

// ─────────────────────────────────────────────
// Native file appenders
// ─────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::HashMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::{BufWriter, Write};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use parking_lot::Mutex;

    struct Appender {
        writer: BufWriter<File>,
        /// Bytes on disk plus bytes buffered — the rotation trigger.
        size: u64,
        path: PathBuf,
    }

    fn appenders() -> &'static Mutex<HashMap<String, Appender>> {
        static APPENDERS: OnceLock<Mutex<HashMap<String, Appender>>> = OnceLock::new();
        APPENDERS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Flush and drop every open appender (called when the configuration — and
    /// so possibly the log directory — changes).
    pub(super) fn close_all() {
        let mut map = appenders().lock();
        for a in map.values_mut() {
            let _ = a.writer.flush();
        }
        map.clear();
    }

    pub(super) fn append(
        dir: &Path,
        name: &str,
        line: &str,
        max_file_size: u64,
        max_files: u32,
        flush_each_line: bool,
    ) -> std::io::Result<()> {
        let mut map = appenders().lock();
        let key = name.to_ascii_lowercase();

        if !map.contains_key(&key) {
            let appender = open(dir, name)?;
            map.insert(key.clone(), appender);
        }
        // Rotation is checked before the write so a line is never split across
        // generations.
        let needs_rotate = {
            let a = map.get(&key).expect("just inserted");
            max_file_size > 0 && a.size + line.len() as u64 > max_file_size && a.size > 0
        };
        if needs_rotate {
            let a = map.remove(&key).expect("just checked");
            rotate(a, max_files)?;
            map.insert(key.clone(), open(dir, name)?);
        }

        let a = map.get_mut(&key).expect("present");
        a.writer.write_all(line.as_bytes())?;
        a.size += line.len() as u64;
        if flush_each_line {
            a.writer.flush()?;
        }
        Ok(())
    }

    /// Open (creating if needed) `<dir>/<name>.log`, writing the CSV header row
    /// when the file is new — matching Lucee, which emits the header once.
    fn open(dir: &Path, name: &str) -> std::io::Result<Appender> {
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.log", name));
        let existing_len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut writer = BufWriter::new(file);
        let mut size = existing_len;
        if existing_len == 0 {
            writer.write_all(super::HEADER.as_bytes())?;
            size += super::HEADER.len() as u64;
        }
        Ok(Appender { writer, size, path })
    }

    /// Size-based rotation. Rolled generations are named `<name>.log.<n>.bak`,
    /// which is what Lucee's resource appender produces — verified by
    /// overflowing a log on Lucee 7.0.4 past the 10 MB default and observing
    /// `rotfill.log.1.bak`. (log4j2's own default would be `<name>.<n>.log`;
    /// Lucee's convention is the one that matters for log tooling parity.)
    /// `.1` is always the most recent roll; older generations shift up and
    /// anything past `max_files` is discarded.
    fn rotate(mut appender: Appender, max_files: u32) -> std::io::Result<()> {
        appender.writer.flush()?;
        drop(appender.writer);
        let path = appender.path;
        let base = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let gen_path = |n: u32| dir.join(format!("{}.{}.bak", base, n));

        if max_files == 0 {
            let _ = fs::remove_file(&path);
            return Ok(());
        }
        let _ = fs::remove_file(gen_path(max_files));
        for n in (1..max_files).rev() {
            let from = gen_path(n);
            if from.exists() {
                let _ = fs::rename(&from, gen_path(n + 1));
            }
        }
        fs::rename(&path, gen_path(1))
    }

    pub(super) fn flush_all() {
        let mut map = appenders().lock();
        for a in map.values_mut() {
            let _ = a.writer.flush();
        }
    }
}
