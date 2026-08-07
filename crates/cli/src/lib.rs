mod engine_cfc_overlay;
mod rewrite;
mod session;
mod socketio;
mod websocket;
/// OpenTelemetry integration (observability Phase 3). Host-only, behind the
/// `obs-otel` feature — the heavy OTLP/reqwest/prometheus deps never reach wasm.
#[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
mod otel;
/// Native CPU/wall-clock sampling profiler (observability Phase 6). Unix-only,
/// behind the `obs-pprof` feature — wraps pprof-rs (SIGPROF).
#[cfg(all(feature = "obs-pprof", unix))]
mod pprof_profile;

/// Sampling heap profiler (`--memprofile`) — runtime-armed global allocator,
/// behind the `memprofile` feature. Public because the binary crate has to
/// install `SamplingAlloc` as `#[global_allocator]`.
#[cfg(all(feature = "memprofile", unix))]
pub mod memprofile;

use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::{Arc, OnceLock, RwLock};

use cfml_codegen::compiler::CfmlCompiler;
use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::logging;
use cfml_common::vfs::{self, Vfs};
use cfml_config::{resolve, RustCfmlConfig};
use cfml_compiler::lexer;
use cfml_compiler::parser::Parser as CfmlParser;
use cfml_compiler::tag_parser;
use cfml_stdlib::builtins::{get_builtin_functions, get_builtins};
use cfml_vm::{CfmlVirtualMachine, ServerState, ThreadHandle, ThreadSeed, compile_file_cached};

// Public re-exports for `--build`-produced native-module crates. A module's
// `register(vm)` function only needs to depend on `rustcfml-cli` to reach
// every type required to register builtins and classes.
pub use cfml_common::dynamic::{CfmlNative, CfmlValue as Value};
pub use cfml_common::vm::{CfmlError, CfmlResult};
pub use cfml_vm::CfmlVirtualMachine as Vm;
// QoQ function registration for native modules (`vm.register_native_qoq_fn`).
pub use cfml_qoq::function::QoQFnKind;
// Re-exported so module authors can construct `Value::strukt(ValueMap::default())`
// without declaring indexmap as a separate dep.
pub use indexmap::IndexMap;

/// An empty `cgi` scope tagged as a Lucee-style magic scope: reading any unset
/// key yields `""` (not null), matching the serve-mode cgi. Used for the
/// no-request CLI/`include` paths so cgi semantics are identical in both modes.
fn empty_magic_cgi() -> CfmlValue {
    let mut m = ValueMap::default();
    m.insert(
        cfml_common::dynamic::EMPTY_DEFAULT_SCOPE_MARKER.to_string(),
        CfmlValue::Bool(true),
    );
    CfmlValue::strukt(m)
}

// ---------------------------------------------------------------------------
// Native-module registrar
// ---------------------------------------------------------------------------
//
// External binaries built via `rustcfml --build` from a project containing a
// `native/` directory call `run_with_registrar` (or `set_registrar` + `run`)
// to inject extra Rust-backed builtins and classes into every VM that the
// runtime constructs. The registrar fires after stdlib registration so it
// can (intentionally) override built-ins if needed.

type Registrar = Box<dyn Fn(&mut CfmlVirtualMachine) + Send + Sync>;
static REGISTRAR: OnceLock<Registrar> = OnceLock::new();

/// Install a callback that is invoked on every VM the runtime constructs,
/// immediately after the standard library has been registered. Calling this
/// more than once per process is a no-op after the first; the first
/// registrar wins.
pub fn set_registrar<F>(registrar: F)
where
    F: Fn(&mut CfmlVirtualMachine) + Send + Sync + 'static,
{
    let _ = REGISTRAR.set(Box::new(registrar));
}

/// Apply the installed registrar (if any) to a freshly-built VM. Public so
/// downstream test harnesses can call it directly when they construct a VM
/// outside of `run()`.
pub fn apply_native_modules(vm: &mut CfmlVirtualMachine) {
    if let Some(r) = REGISTRAR.get() {
        r(vm);
    }
}

/// Convenience wrapper: install the registrar and then enter the standard
/// CLI/serve entry point. The Model A-generated `main.rs` collapses to:
///
/// ```ignore
/// fn main() {
///     rustcfml_cli::run_with_registrar(|vm| {
///         my_native_hello::register(vm);
///     });
/// }
/// ```
pub fn run_with_registrar<F>(registrar: F)
where
    F: Fn(&mut CfmlVirtualMachine) + Send + Sync + 'static,
{
    set_registrar(registrar);
    run();
}

#[derive(Parser, Debug)]
#[command(name = "rustcfml")]
#[command(about = "A CFML interpreter written in Rust", long_about = None)]
struct Args {
    /// The CFML file to execute
    #[arg(default_value = "")]
    file: String,

    /// Execute code from command line
    #[arg(short, long)]
    code: Option<String>,

    /// Enable debug output
    #[arg(short, long)]
    debug: bool,

    /// Run in interactive REPL mode
    #[arg(short, long)]
    repl: bool,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Show version information
    #[arg(long)]
    version: bool,

    /// Print RustCFML's license plus the third-party attribution notices for
    /// every crate linked into this binary, then exit
    #[arg(long)]
    licenses: bool,

    /// Start web server with document root (default: current directory)
    #[arg(long, num_args = 0..=1, default_missing_value = ".")]
    serve: Option<String>,

    /// Server port (default: 8500)
    #[arg(long, default_value = "8500")]
    port: u16,

    /// Bind the web server to a Unix domain socket instead of a TCP port.
    /// With no value, defaults to /run/rustcfml.sock. When set, --port is
    /// ignored (the socket wins). Unix-only.
    #[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "/run/rustcfml.sock")]
    socket: Option<String>,

    /// Explicit path to the server-baseline `.cfconfig.json`. When set, it is
    /// loaded instead of searching webroot/cwd/exe-dir. Also honored via the
    /// `CFCONFIG` env var (Lucee/CommandBox parity). Per-application
    /// `.cfconfig.json` files beside an `Application.cfc` overlay this baseline.
    #[arg(long, value_name = "PATH")]
    cfconfig: Option<String>,

    /// Use single-threaded async runtime (lower memory, lower concurrency)
    #[arg(long)]
    single_threaded: bool,

    /// Enable production mode: cache Application.cfc resolution, URL→file
    /// resolution, and bytecode cache entries permanently (no mtime checks,
    /// no readdir per request). Restart the server to pick up file changes.
    /// Also honored via `RUSTCFML_PRODUCTION=1`.
    #[arg(long)]
    production: bool,

    /// Build a self-contained binary: embed a CFML app into a single executable
    /// Usage: rustcfml --build <app-dir> [-o output-binary] [--mode serve|cli]
    #[arg(long, value_name = "APP_DIR")]
    build: Option<String>,

    /// Output path for the built binary (default: ./app)
    #[arg(short, long, default_value = "app")]
    output: String,

    /// Build mode: "serve" for web server (default), "cli" for command-line tool
    #[arg(long, default_value = "serve")]
    mode: String,

    /// Entry point for CLI mode (default: main.cfm)
    #[arg(long, default_value = "main.cfm")]
    entry: String,

    /// Disable the optional Cranelift JIT at runtime. Equivalent to setting
    /// `RUSTCFML_JIT=0` in the environment — the interpreter handles every
    /// op and the JIT engine isn't initialised. Has no effect on builds
    /// produced without `--features jit`.
    #[arg(long)]
    no_jit: bool,

    /// Override the JIT hotness threshold (number of invocations before a
    /// candidate function or loop is compiled). Equivalent to setting
    /// `RUSTCFML_JIT_THRESHOLD=N`. Defaults to 50.
    #[arg(long, value_name = "N")]
    jit_threshold: Option<u32>,

    /// After execution, print JIT statistics to stderr: number of whole
    /// functions compiled and number of OSR loop bodies compiled. Useful
    /// for diagnostics, threshold tuning, and confirming a hot path
    /// actually engaged the JIT. No effect when the JIT feature is off.
    #[arg(long)]
    jit_stats: bool,

    /// Before executing, walk the compiled bytecode and print an
    /// Option-γ coverage report to stderr: per-op classification
    /// (supported / boxed-promising / hopeless) and the fraction of
    /// functions that would become admissible once polymorphic
    /// tag-pointer values land (see `JIT_POLY_DESIGN.md`). Forward-
    /// looking diagnostic only — does not change execution behaviour.
    #[arg(long)]
    jit_coverage: bool,

    /// Native CPU/wall-clock sampling profiler (observability Phase 6). Samples
    /// the Rust call stack at ~100 Hz and, on exit, writes `rustcfml-profile.svg`
    /// (flamegraph) + `rustcfml-profile.pb` (pprof protobuf, loadable in
    /// `go tool pprof` / speedscope / Pyroscope). Works for a one-shot run
    /// (profiles that run) and with `--serve` (process-wide aggregate over all
    /// requests; written on graceful Ctrl+C shutdown). Requires a build with
    /// `--features obs-pprof`; Unix-only.
    #[arg(long)]
    profile: bool,

    /// Sampling heap profiler. Arms a global allocator that samples roughly one
    /// allocation per 256 KiB (override with `RUSTCFML_MEMPROFILE_RATE`) and
    /// attributes live bytes to the Rust stack that allocated them. Writes
    /// `rustcfml-memprofile-N-inuse.{pb,folded}` (where the RSS is) and
    /// `-alloc.{pb,folded}` (what is churning), both loadable in `go tool pprof`.
    /// In `--serve` mode, `kill -USR2 <pid>` dumps on demand without stopping the
    /// server; a final dump is written on graceful Ctrl+C shutdown. Requires a
    /// build with `--features memprofile`; Unix-only.
    #[arg(long)]
    memprofile: bool,
}

/// Process-global flag set by the `--jit-stats` CLI option. Polled after
/// each VM run (`execute_with_session_handling`) to print a one-line
/// diagnostic to stderr. Static-atomic so we don't have to thread an extra
/// param through every nested call site (compile_and_run → … → the per-VM
/// execute) just for a debug knob; the JIT itself is also a process-global
/// per-VM init, so the scope matches.
static JIT_STATS_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Process-global flag set by `--jit-coverage`. Triggers the Option-γ
/// coverage scan + render after compile, before run.
static JIT_COVERAGE_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Encapsulates the full response from CFML execution, including HTTP metadata.
struct CfmlResponse {
    output: String,
    response_headers: Vec<(String, String)>,
    response_status: Option<(u16, String)>,
    response_content_type: Option<String>,
    response_body: Option<CfmlValue>,
    redirect_url: Option<String>,
    session_id: Option<String>,
    /// True when a session record was actually created/minted this request
    /// (eager start, or the first session write under the lazy default). Drives
    /// whether a `CFID` cookie is emitted — under lazy sessions, a request that
    /// only reads session mints nothing and gets no cookie.
    session_record_created: bool,
    /// Resolved `this.sessioncookie` attributes, used to render the session
    /// `Set-Cookie` header (Secure/HttpOnly/SameSite/Domain/Path).
    session_cookie_policy: cfml_common::session_cookie::SessionCookiePolicy,
    /// Only meaningful on the onMissingTemplate path: true when an
    /// Application.cfc `onMissingTemplate` handler ran and handled the request
    /// (returned anything but `false`). When false the embedder emits its
    /// default 404 instead of this response's output.
    missing_template_handled: bool,
}

/// Error from CFML execution, carrying any output generated before the error.
struct CfmlRunError {
    output: String,
    message: String,
}

/// Standard CLI/serve entry point — what `rustcfml` itself runs. Spawns a
/// big-stack thread for the VM and dispatches to embedded-app / REPL / file /
/// `--serve` / `--build` based on CLI args. Use `run_with_registrar` instead
/// if you need to register native modules first.
pub fn run() {
    // Spawn a thread with a large stack (64 MB) so deep recursion in the VM
    // doesn't blow the default ~8 MB main-thread stack (especially in debug builds).
    const STACK_SIZE: usize = 64 * 1024 * 1024;
    let builder = std::thread::Builder::new().stack_size(STACK_SIZE);
    let handler = builder.spawn(real_main).expect("failed to spawn main thread");
    if let Err(e) = handler.join() {
        eprintln!("Fatal: {:?}", e);
        exit(1);
    }
}

// RustCFML's own license and the generated third-party attribution notice are
// compiled INTO the binary. Rationale: these binaries are distributed as bare
// single files (see .github/workflows/release.yml) and are routinely copied out
// of the release page, out of a container image, or out of a `--build` bundle.
// A sibling LICENSE/THIRD-PARTY.txt asset does not survive that; an embedded
// one does, so the notices required by the MIT/BSD/ISC/Apache-2.0 terms of our
// ~560 statically linked dependencies always travel with the artifact.
//
// THIRD-PARTY.txt is generated and committed — regenerate with
// `scripts/gen-licenses.sh` after any dependency change.
const OWN_LICENSE: &str = include_str!("../../../LICENSE");
const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD-PARTY.txt");

fn print_licenses() {
    use std::io::Write;

    let body = format!(
        "RustCFML v{}\n\n{}\n\n{}\n",
        env!("CARGO_PKG_VERSION"),
        OWN_LICENSE.trim_end(),
        THIRD_PARTY_NOTICES.trim_end()
    );

    // Written with write_all rather than println! so a closed pipe is not a
    // panic. This output is ~13k lines, so `--licenses | head` and
    // `--licenses | less` (quit before the end) are the normal ways to read it,
    // and both close the pipe early. Rust ignores SIGPIPE process-wide, so
    // println! would hit EPIPE and panic with "failed printing to stdout".
    // Restoring SIG_DFL globally is not an option — serve mode relies on socket
    // writes returning EPIPE instead of killing the process when a client
    // disconnects.
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    match handle.write_all(body.as_bytes()).and_then(|_| handle.flush()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(e) => {
            eprintln!("Error writing licenses: {}", e);
            exit(1);
        }
    }
}

fn real_main() {
    // Check for embedded archive — if present, run as self-contained app
    if let Some(files) = vfs::extract_embedded_archive() {
        run_embedded_app(files);
        return;
    }

    let args = Args::parse();

    if args.version {
        println!("RustCFML v{}", env!("CARGO_PKG_VERSION"));
        exit(0);
    }

    if args.licenses {
        print_licenses();
        exit(0);
    }

    if args.verbose {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    }

    // JIT control flags (mirror the existing RUSTCFML_JIT / RUSTCFML_JIT_THRESHOLD
    // env vars). Must be set BEFORE the first `CfmlVirtualMachine::new` since
    // `JitEngine::maybe_new` reads these at construction time.
    if args.no_jit {
        // SAFETY: single-threaded startup; no races with other env readers.
        unsafe {
            std::env::set_var("RUSTCFML_JIT", "0");
        }
    }
    if let Some(n) = args.jit_threshold {
        // SAFETY: same — pre-VM-init.
        unsafe {
            std::env::set_var("RUSTCFML_JIT_THRESHOLD", n.to_string());
        }
    }
    if args.jit_stats {
        JIT_STATS_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if args.jit_coverage {
        JIT_COVERAGE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
        // Also propagate via env var so the in-cfml-vm compile path (which
        // doesn't see the CLI args struct directly) can dump too. Set
        // BEFORE the first VM init for the same reason as RUSTCFML_JIT.
        unsafe {
            std::env::set_var("RUSTCFML_JIT_COVERAGE", "1");
        }
    }

    // Handle --build <app-dir>
    if let Some(ref app_dir) = args.build {
        let mode = args.mode.to_lowercase();
        if mode != "serve" && mode != "cli" {
            eprintln!("Error: --mode must be 'serve' or 'cli'");
            exit(1);
        }
        build_self_contained(app_dir, &args.output, &mode, &args.entry);
        return;
    }

    if let Some(ref doc_root) = args.serve {
        let mut doc_root = PathBuf::from(doc_root);
        if !doc_root.is_dir() {
            eprintln!("Error: Document root is not a directory: {}", doc_root.display());
            exit(1);
        }
        let production = args.production
            || std::env::var("RUSTCFML_PRODUCTION").as_deref() == Ok("1");

        // Server-baseline .cfconfig.json. An explicit `--cfconfig <path>` (or the
        // `CFCONFIG` env var) wins; otherwise search webroot → cwd → exe dir.
        // Per-application `.cfconfig.json` files (beside an Application.cfc) are
        // discovered per-request and overlaid on top of this baseline.
        let explicit_cfconfig = args
            .cfconfig
            .clone()
            .or_else(|| std::env::var("CFCONFIG").ok().filter(|s| !s.is_empty()));
        let mut cfconfig = match explicit_cfconfig {
            Some(path) => match RustCfmlConfig::from_file(std::path::Path::new(&path)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error loading --cfconfig {}: {}", path, e);
                    exit(1);
                }
            },
            None => {
                let mut search_paths: Vec<PathBuf> = vec![doc_root.clone()];
                if let Ok(cwd) = std::env::current_dir() {
                    search_paths.push(cwd);
                }
                if let Some(dir) = resolve::exe_dir() {
                    search_paths.push(dir);
                }
                match RustCfmlConfig::load(&search_paths) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Error loading .cfconfig.json: {}", e);
                        exit(1);
                    }
                }
            }
        };

        // Logging: --verbose and RUST_LOG keep priority. Otherwise apply
        // logging.level from cfconfig (default "warn"). logsDirectory and
        // format are accepted by the schema but not yet wired — log a
        // one-line warning if the user set them so the silence isn't
        // mysterious. logger-name overrides are merged into the same
        // env_logger filter string.
        if !args.verbose && std::env::var("RUST_LOG").is_err() {
            let mut filter = cfconfig.logging.level.clone();
            if filter.is_empty() {
                filter = "warn".to_string();
            }
            for (name, lcfg) in cfconfig.logging.loggers.iter() {
                if !lcfg.level.is_empty() {
                    filter.push_str(&format!(",{}={}", name, lcfg.level));
                }
            }
            let _ = env_logger::Builder::from_env(
                env_logger::Env::default().default_filter_or(filter),
            )
            .try_init();
            if !cfconfig.logging.format.is_empty() && cfconfig.logging.format != "text" {
                log::warn!(
                    "cfconfig logging.format='{}' is not yet supported — using text",
                    cfconfig.logging.format
                );
            }
        }

        // The listening port is a server/environment concern, never a cfconfig
        // setting (cfconfig is application-level). It comes solely from `--port`
        // (clap default 8500).
        let port = args.port;
        // --socket overrides --port (the socket wins). A bare `--socket` with no
        // path falls back to the clap default /run/rustcfml.sock.
        let socket = args.socket.as_ref().map(PathBuf::from);
        // server.webroot from config only applies when --serve had no path.
        if doc_root == PathBuf::from(".") && !cfconfig.server.webroot.is_empty() {
            doc_root = PathBuf::from(&cfconfig.server.webroot);
            if !doc_root.is_dir() {
                eprintln!("Error: cfconfig server.webroot is not a directory: {}", doc_root.display());
                exit(1);
            }
        }

        // `<cflog>`/`writeLog()` file appenders — resolved against the final
        // webroot, so this has to follow the server.webroot fallback above.
        configure_cfml_logging(&mut cfconfig, Some(&doc_root));

        run_server(
            &doc_root,
            port,
            socket,
            args.debug,
            args.single_threaded,
            // Overlay the engine-bundled compat CFCs (socket.io trio +
            // Lucee's `new Query()` builder) so they resolve without the
            // user shipping them.
            Arc::new(engine_cfc_overlay::EngineCfcOverlay::new(vfs::real_fs())),
            false,
            production,
            Arc::new(cfconfig),
            args.profile,
            args.memprofile,
        );
        return;
    }

    if args.repl {
        run_repl(args.debug);
        return;
    }

    if let Some(code) = args.code {
        execute_code(&code, args.debug);
        return;
    }

    if args.file.is_empty() {
        println!("RustCFML v{}", env!("CARGO_PKG_VERSION"));
        println!("Usage: rustcfml <file.cfm|.cfc>");
        println!("       rustcfml -c \"<code>\"");
        println!("       rustcfml -r (REPL mode)");
        println!("       rustcfml --serve [path] [--port 8500 | --socket [PATH]]");
        println!("       rustcfml --build <app-dir> [-o output]");
        println!("       rustcfml --help");
        exit(0);
    }

    let path = PathBuf::from(&args.file);
    if !path.exists() {
        eprintln!("Error: File not found: {}", args.file);
        exit(1);
    }

    // Native sampling profiler (Phase 6): arm it around the whole run when
    // `--profile` is set. Held until `finish()` writes the flamegraph + pprof.
    #[cfg(all(feature = "obs-pprof", unix))]
    let _profiler = if args.profile {
        pprof_profile::start("rustcfml-profile")
    } else {
        None
    };
    #[cfg(not(all(feature = "obs-pprof", unix)))]
    if args.profile {
        eprintln!("--profile requires a build with `--features obs-pprof` (Unix only)");
    }

    // Sampling heap profiler: arm around the run, dump once it completes.
    #[cfg(all(feature = "memprofile", unix))]
    if args.memprofile {
        memprofile::arm("rustcfml-memprofile");
    }
    #[cfg(not(all(feature = "memprofile", unix)))]
    if args.memprofile {
        eprintln!("--memprofile requires a build with `--features memprofile` (Unix only)");
    }

    execute_file(&path, args.debug);

    #[cfg(all(feature = "memprofile", unix))]
    if args.memprofile {
        memprofile::finish("rustcfml-memprofile");
    }

    #[cfg(all(feature = "obs-pprof", unix))]
    if let Some(session) = _profiler {
        session.finish();
    }
}

fn execute_file(path: &PathBuf, debug: bool) {
    let source = fs::read_to_string(path).expect("Failed to read file");
    execute_code_with_file(&source, debug, Some(path.to_string_lossy().to_string()));
}

fn execute_code(source: &str, debug: bool) {
    execute_code_with_file(source, debug, None);
}

/// Resolve cfconfig in CLI (non-serve) mode. Searches the entry file's
/// directory, then cwd, then exe dir. Returns `None` so the VM can still
/// operate without a server_state attached.
fn load_cli_cfconfig(source_file: &Option<String>) -> RustCfmlConfig {
    let mut search: Vec<PathBuf> = Vec::new();
    if let Some(ref f) = source_file {
        if let Some(parent) = std::path::Path::new(f).parent() {
            search.push(parent.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        search.push(cwd);
    }
    if let Some(d) = resolve::exe_dir() {
        search.push(d);
    }
    RustCfmlConfig::load(&search).unwrap_or_default()
}

/// Install the CFML log-appender configuration (`<cflog>` / `writeLog()`).
///
/// Directory precedence: `logging.logsDirectory` from cfconfig, else
/// `<webroot>/logs` in serve mode, else `./logs` under the CLI. Lucee's
/// equivalent is the server context's `logs/` directory; keeping ours relative
/// to the webroot means a deployed app's logs sit beside the app.
/// The resolved directory is written back into `logging.logsDirectory`, so CFML
/// can read the *effective* location off `server.cfconfig.logging.logsDirectory`
/// rather than the (usually empty) configured value.
fn configure_cfml_logging(cfconfig: &mut RustCfmlConfig, webroot: Option<&std::path::Path>) {
    let lg = &cfconfig.logging;
    let directory = if !lg.logs_directory.is_empty() {
        PathBuf::from(&lg.logs_directory)
    } else if let Some(root) = webroot {
        root.join("logs")
    } else {
        PathBuf::from("logs")
    };

    let default_level = if lg.cfml_level.is_empty() {
        Some(logging::LogLevel::Trace)
    } else {
        match logging::parse_level_threshold(&lg.cfml_level) {
            Ok(l) => l,
            Err(()) => {
                eprintln!(
                    "Warning: cfconfig logging.cfmlLevel='{}' is not a valid level \
                     (trace|debug|info|warn|error|fatal|off) — logging everything",
                    lg.cfml_level
                );
                Some(logging::LogLevel::Trace)
            }
        }
    };

    let mut levels = std::collections::HashMap::new();
    for (name, lcfg) in lg.loggers.iter() {
        if lcfg.level.is_empty() {
            continue;
        }
        // A `loggers` entry may name an engine (Rust) log target rather than a
        // CFML log; those levels are `log`-crate spellings that don't map here.
        // Skip silently — the entry is still honoured by the env_logger filter.
        if let Ok(threshold) = logging::parse_level_threshold(&lcfg.level) {
            levels.insert(name.to_ascii_lowercase(), threshold);
        }
    }

    logging::configure(logging::LoggingConfig {
        directory: Some(directory.clone()),
        default_level,
        levels,
        max_file_size: lg.max_file_size,
        max_files: lg.max_files,
        echo_stderr: lg.echo_to_stderr || std::env::var("RUSTCFML_LOG_STDERR").is_ok(),
        flush_each_line: lg.flush_each_line,
    });

    // Absolute where we can, so a CFML consumer doesn't have to guess the cwd.
    let resolved = std::fs::canonicalize(&directory)
        .or_else(|_| std::env::current_dir().map(|cwd| cwd.join(&directory)))
        .unwrap_or(directory);
    cfconfig.logging.logs_directory = resolved.to_string_lossy().to_string();
}

fn execute_code_with_file(source: &str, debug: bool, source_file: Option<String>) {
    // CLI mode: load .cfconfig.json once, attach via a minimal ServerState so
    // the VM picks up runtime knobs, datasource registry, and security flags.
    // Without this, CFML tests can't observe cfconfig effects.
    let mut cfconfig_owned = load_cli_cfconfig(&source_file);
    configure_cfml_logging(&mut cfconfig_owned, None);
    let cfconfig = Arc::new(cfconfig_owned);
    populate_datasource_registry(&cfconfig);
    populate_default_mail_server(&cfconfig);
    cfml_stdlib::builtins::set_security_flags(cfml_stdlib::builtins::SecurityFlags {
        csrf_enabled: cfconfig.security.csrf_enabled,
        secure_json: cfconfig.security.secure_json,
        secure_json_prefix: cfconfig.security.secure_json_prefix.clone(),
    });
    let mut server_state = ServerState::with_config(false, cfconfig.clone());
    // Sampling profiler in CLI runs: no watchdog thread, so threshold-based
    // auto-sampling never fires, but `profileNow()` still captures synchronously
    // and `getRequestProfile()` reports it. Armed only when config enables it.
    if cfconfig.observability.enabled && cfconfig.observability.profiler.enabled {
        let p = &cfconfig.observability.profiler;
        server_state.profiler = Some(Arc::new(cfml_vm::profiler::ProfilerHub::new(
            p.threshold_ms,
            p.interval_ms,
            p.max_samples,
        )));
    }
    let cli_vfs: Arc<dyn vfs::Vfs> =
        Arc::new(engine_cfc_overlay::EngineCfcOverlay::new(vfs::real_fs()));
    let result = compile_and_run(source, debug, source_file, ValueMap::default(), Some(&server_state), None, None, cli_vfs, false, None, false, false);
    // Buffered `<cflog>` lines must reach disk before we return or `exit(1)`
    // — the error path below never unwinds, so no Drop guard would run.
    logging::flush_all();
    match result {
        Ok(response) => {
            if !response.output.is_empty() {
                print!("{}", response.output);
            }
        }
        Err(e) => {
            if !e.output.is_empty() {
                print!("{}", e.output);
            }
            eprintln!("{}", e.message);
            exit(1);
        }
    }
}

/// Compile and execute CFML source, returning output as a String.
/// `extra_globals` are injected into vm.globals before execution (e.g. web scopes).
fn compile_and_run_with_session(
    source: &str,
    debug: bool,
    source_file: Option<String>,
    extra_globals: ValueMap,
    server_state: Option<&ServerState>,
    http_request_data: Option<CfmlValue>,
    session_id: Option<String>,
    vfs: Arc<dyn Vfs>,
    sandbox: bool,
) -> Result<CfmlResponse, CfmlRunError> {
    compile_and_run(source, debug, source_file, extra_globals, server_state, http_request_data, session_id, vfs, sandbox, None, true, true)
}

/// Run the Application.cfc lifecycle for a requested template that does NOT
/// exist on disk, so an `onMissingTemplate` handler can intercept it. There is
/// no target page to compile — the VM runs onApplicationStart/onSessionStart
/// then onMissingTemplate(targetPage). `would_be_path` is the filesystem path
/// the template would have occupied (used to discover the governing
/// Application.cfc and resolve mappings); `target_page` is the web-root-relative
/// path handed to the handler. Inspect `missing_template_handled` on the result.
#[allow(clippy::too_many_arguments)]
fn run_missing_template(
    would_be_path: String,
    target_page: String,
    extra_globals: ValueMap,
    server_state: Option<&ServerState>,
    http_request_data: Option<CfmlValue>,
    session_id: Option<String>,
    vfs: Arc<dyn Vfs>,
    sandbox: bool,
) -> Result<CfmlResponse, CfmlRunError> {
    compile_and_run("", false, Some(would_be_path), extra_globals, server_state, http_request_data, session_id, vfs, sandbox, Some(target_page), true, true)
}

/// Register the standard runtime fixtures onto a fresh VM: builtins, builtin
/// functions, any native modules registered via `set_registrar` /
/// `run_with_registrar`, and the DB transaction/query function pointers.
///
/// Shared by per-request execution (`compile_and_run`) and by spawned
/// `cfthread` child VMs so both get byte-for-byte identical wiring.
fn register_vm_runtime(vm: &mut CfmlVirtualMachine) {
    for (name, value) in get_builtins() {
        vm.globals.insert(name, value);
    }
    for (name, func) in get_builtin_functions() {
        vm.builtins.insert(name, func);
    }
    vm.refresh_builtin_index();
    apply_native_modules(vm);
    vm.txn_begin = Some(cfml_stdlib::builtins::txn_begin_boxed);
    vm.txn_commit = Some(cfml_stdlib::builtins::txn_commit_boxed);
    vm.txn_rollback = Some(cfml_stdlib::builtins::txn_rollback_boxed);
    vm.txn_savepoint = Some(cfml_stdlib::builtins::txn_savepoint_boxed);
    vm.txn_release_savepoint = Some(cfml_stdlib::builtins::txn_release_savepoint_boxed);
    vm.txn_rollback_to_savepoint = Some(cfml_stdlib::builtins::txn_rollback_to_savepoint_boxed);
    vm.txn_execute = Some(cfml_stdlib::builtins::txn_execute_boxed);
    vm.query_execute_fn = Some(cfml_stdlib::builtins::fn_query_execute);
    // Real-OS-thread cfthread spawner. The VM only uses this when its
    // `real-threads` feature is on (default); injecting it unconditionally is
    // harmless when the feature is off (the VM ignores it and runs inline).
    vm.thread_spawn_fn = Some(spawn_cfthread);
}

/// Stack size for spawned `cfthread` OS threads. Matches the main thread's
/// 64 MB so deeply-recursive CFML in a thread body can't blow the default
/// ~8 MB stack. This is virtual address space, not committed memory.
const CFTHREAD_STACK_SIZE: usize = 64 * 1024 * 1024;

/// Spawn a `cfthread` body on a real OS thread. Builds a fresh child VM from
/// the seed (same runtime wiring as a request VM, via `register_vm_runtime`),
/// runs the body, and reports the `ThreadResult` back over a channel. The
/// parent joins on the returned handle. Registered as the VM's
/// `thread_spawn_fn` (a plain `fn`, so it coerces to the fn-pointer type).
fn spawn_cfthread(seed: ThreadSeed) -> ThreadHandle {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = seed.cancel_flag.clone();
    let join = std::thread::Builder::new()
        .stack_size(CFTHREAD_STACK_SIZE)
        .spawn(move || {
            let mut vm = CfmlVirtualMachine::new(seed.program.clone());
            register_vm_runtime(&mut vm);
            let (closure, attributes) = vm.apply_thread_seed(seed);
            let result = vm.run_thread_body(&closure, attributes, &ValueMap::default());
            // Receiver may be gone if the parent never joined; ignore.
            let _ = tx.send(result);
        })
        .expect("cfthread: failed to spawn OS thread");
    ThreadHandle {
        name: String::new(),
        rx,
        cancel,
        join: Some(join),
        result: None,
    }
}

/// `--jit-stats` dump (jit builds): cumulative per-worker compile counts.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
fn dump_jit_stats(vm: &CfmlVirtualMachine) {
    if JIT_STATS_REQUESTED.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!(
            "jit-stats: fn_compiled={} osr_compiled={}",
            vm.jit_compiled_count(),
            vm.osr_compiled_count()
        );
    }
}

/// Whether serve-mode cross-request JIT persistence is enabled. On by default;
/// set `RUSTCFML_JIT_PERSIST=0` (or false/off/no) to fall back to a per-request
/// engine without a rebuild.
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
fn jit_persist_enabled() -> bool {
    std::env::var("RUSTCFML_JIT_PERSIST")
        .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
        .unwrap_or(true)
}

#[allow(clippy::too_many_arguments)]
fn compile_and_run(
    source: &str,
    debug: bool,
    source_file: Option<String>,
    extra_globals: ValueMap,
    server_state: Option<&ServerState>,
    http_request_data: Option<CfmlValue>,
    session_id: Option<String>,
    vfs: Arc<dyn Vfs>,
    sandbox: bool,
    missing_template: Option<String>,
    persist_jit: bool,
    web_context: bool,
) -> Result<CfmlResponse, CfmlRunError> {
    // Connection-per-request DB isolation. Serve-mode requests run on reused
    // tokio blocking threads, and each request holds one pooled DB connection per
    // datasource (so session/user state persists across a request's statements —
    // see REQUEST_MYSQL_CONNS). Release those held connections at the request
    // boundary so their session state (SET foreign_key_checks / sql_mode / …) is
    // reset before the connection serves the next request (GitHub #275) and does
    // not corrupt it (Masa admin boot). Release up-front too, so a prior request
    // that died mid-flight on this thread can't leak its connection into this one.
    // The drop guard covers every early return and panic.
    cfml_stdlib::builtins::release_request_db_conns();
    struct DbConnRequestGuard;
    impl Drop for DbConnRequestGuard {
        fn drop(&mut self) {
            cfml_stdlib::builtins::release_request_db_conns();
        }
    }
    let _db_conn_guard = DbConnRequestGuard;

    // onMissingTemplate path: there is no target page to compile, so use an
    // empty program. execute_with_lifecycle never runs it — it dispatches to
    // the Application.cfc onMissingTemplate handler and returns.
    let program = if missing_template.is_some() {
        let compiler = CfmlCompiler::new();
        let ast = CfmlParser::new(String::new()).parse().expect("empty source parses");
        compiler.compile(ast)
    }
    // In serve mode with a source file, use the bytecode cache to skip recompilation
    else if !debug && source_file.is_some() && server_state.is_some() {
        let path = source_file.as_ref().unwrap();
        let cache = &server_state.unwrap().bytecode_cache;
        compile_file_cached(path, Some(cache), vfs.as_ref()).map_err(|e| CfmlRunError { output: String::new(), message: format!("{}", e) })?
    } else {
        // CLI mode / inline code / debug: full pipeline
        // Strip shebang line if present (e.g. #!/usr/bin/env rustcfml)
        let source = if source.starts_with("#!") {
            source.split_once('\n').map_or("", |(_shebang, rest)| rest)
        } else {
            source
        };

        // Pre-process: convert CFML tags to script if needed
        let source = if tag_parser::has_cfml_tags(source) {
            let converted = tag_parser::tags_to_script_checked(source)
                .map_err(|msg| CfmlRunError { output: String::new(), message: msg })?;
            if debug {
                println!("=== TAG CONVERSION ===");
                println!("{}", converted);
                println!();
            }
            converted
        } else {
            source.to_string()
        };
        let source = source.as_str();

        // Lexical analysis
        let tokens = lexer::tokenize(source.to_string());

        if debug {
            println!("=== TOKENS ===");
            for (i, tok) in tokens.iter().enumerate() {
                println!("{:3}: {:?}", i, tok.token);
            }
            println!();
        }

        // Parse to AST
        let mut parser = CfmlParser::new(source.to_string());
        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(e) => {
                return Err(CfmlRunError {
                    output: String::new(),
                    message: format!("Parse Error [line {}, col {}]: {}", e.line, e.column, e.message),
                });
            }
        };

        if debug {
            println!("=== AST ===");
            println!("{:#?}", ast);
            println!();
        }

        // Compile to bytecode
        let compiler = CfmlCompiler::new();
        let program = compiler.compile(ast);

        // --jit-coverage: dump the Option-γ forecast before running. Cheap,
        // pure read of the bytecode; doesn't change execution. Stderr so
        // it doesn't interleave with the program's stdout.
        #[cfg(feature = "jit")]
        if JIT_COVERAGE_REQUESTED.load(std::sync::atomic::Ordering::Relaxed) {
            let report = cfml_vm::jit::coverage::scan_program(&program);
            eprintln!("{}", report.render());
        }

        if debug {
            println!("=== BYTECODE ===");
            for func in &program.functions {
                println!("Function: {} (params: {:?})", func.name, func.params);
                for (i, instr) in func.instructions.iter().enumerate() {
                    match instr {
                        cfml_codegen::BytecodeOp::LineInfo(line, col) => {
                            println!("        ; line {}:{}", line, col);
                        }
                        _ => {
                            println!("  {:3}: {:?}", i, instr);
                        }
                    }
                }
            }
            println!();
        }

        program
    };

    // Execute
    let mut vm = CfmlVirtualMachine::new(program);
    vm.vfs = vfs;
    vm.sandbox = sandbox;
    vm.base_template_path = source_file.clone();
    vm.source_file = source_file;

    // Register builtins, builtin functions, native modules, and the DB
    // transaction/query function pointers. Shared with spawned cfthread child
    // VMs so both get identical runtime wiring.
    register_vm_runtime(&mut vm);

    // Ensure web scopes always exist (CFML guarantees url/cgi/form are always defined)
    vm.globals.entry("url".to_string()).or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals.entry("cgi".to_string()).or_insert_with(empty_magic_cgi);
    vm.globals.entry("form".to_string()).or_insert_with(|| CfmlValue::strukt(ValueMap::default()));

    // Inject extra globals (web scopes, etc.) — overrides defaults above in serve mode
    for (name, value) in extra_globals {
        vm.globals.insert(name, value);
    }

    // Web request (serve mode) → writeDump emits its HTML widget; CLI runs
    // emit a plain-text tree.
    vm.web_context = web_context;

    // Wire up server state if provided (for --serve mode)
    if let Some(ss) = server_state {
        // Overlay .cfconfig.json runtime knobs onto the VM. ServerState owns
        // the Arc; we only need a borrow long enough to copy values across.
        vm.apply_cfconfig(&ss.cfconfig);
        vm.server_state = Some(ss.clone());
    }

    // Wire up HTTP request data if provided
    vm.http_request_data = http_request_data;

    // Wire up session ID
    vm.session_id = session_id;

    // onMissingTemplate: hand the VM the web-root-relative requested path so
    // execute_with_lifecycle dispatches to the handler instead of a target page.
    vm.missing_template = missing_template;

    // Classic CF debug footer (Phase 1): evaluate the activation gates now that
    // web scopes + cfconfig are in place, and install the per-request collector
    // when they pass. A request that won't show debug output collects nothing.
    vm.maybe_install_debug_collector();

    // Sampling profiler (Phase 2): register this request with the profiler hub
    // (a no-op when the profiler is off). A watchdog thread will ask it to
    // sample its own call stack if it runs past the threshold.
    vm.maybe_arm_profiler();

    // OpenTelemetry (Phase 3): open the request root span + install the
    // per-request span-building observer. No-op (returns None) unless OTel was
    // initialised at server start, so CLI/one-shot runs pay nothing.
    #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
    let otel_req = otel::begin_request(&mut vm);
    #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
    let otel_start = std::time::Instant::now();

    // Start logging this request's container allocations so the request-boundary
    // cycle collector can reclaim any reference cycles it built (the serve-mode
    // leak fix). Armed only in serve mode; a no-op otherwise. See cfml_common::cycle_gc.
    if cfml_common::cycle_gc::is_armed() {
        cfml_common::cycle_gc::enable();
    }

    // Run the lifecycle. In serve mode (`persist_jit`), adopt this worker
    // thread's persistent JIT engine for the duration via `JitLease` so
    // compiled native code + hotness counters accumulate across requests
    // instead of being rebuilt cold every request. The lease returns the
    // engine to the thread-local on scope exit (incl. panic/early return).
    // The `--jit-stats` read must happen while the engine is still in the VM
    // (before the lease drops), so it reports the cumulative per-worker count.
    let result;
    #[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
    {
        if persist_jit && jit_persist_enabled() {
            let mut lease = cfml_vm::JitLease::new(&mut vm);
            result = lease.vm().execute_with_lifecycle();
            dump_jit_stats(lease.vm());
        } else {
            result = vm.execute_with_lifecycle();
            dump_jit_stats(&vm);
        }
    }
    #[cfg(not(all(feature = "jit", not(target_arch = "wasm32"))))]
    {
        let _ = persist_jit;
        result = vm.execute_with_lifecycle();
        if JIT_STATS_REQUESTED.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("jit-stats: JIT feature not built in (rebuild with --features jit)");
        }
    }

    // Flush any buffered <cfhtmlhead>/<cfhtmlbody> content into the output
    // before it is consumed.
    vm.finalize_html_injections();

    // Render the debug footer (if a collector was installed and the page is a
    // renderable HTML response not suppressed by <cfsetting showDebugOutput>).
    // Appends to the output buffer, so it flows into the response body below.
    vm.maybe_render_debug_footer();

    // Sampling profiler (Phase 2): fold this request's samples into a call tree,
    // publish it to the hub for the /__rustcfml/profiler admin endpoint, and
    // deregister. A no-op when nothing was sampled.
    vm.finish_profiler();

    // OpenTelemetry (Phase 3): close the request root span + record RED metrics.
    // Reads the outcome BEFORE the redirect/abort remap below so the status code
    // reflects redirect (302) / abort (200) / error (500) accurately.
    #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
    if let Some((ref root, ref route)) = otel_req {
        // Coarse error label — the message is high-cardinality (never a metric
        // label); the exception *type* is recorded on the span by on_error.
        let (status, err): (u16, Option<&str>) = match &result {
            Ok(_) => (200, None),
            Err(e) if e.message == "__cflocation_redirect" => (302, None),
            Err(e) if e.message == "__cfabort" => (200, None),
            Err(_) => (500, Some("error")),
        };
        otel::end_request(root, route, status, otel_start.elapsed().as_secs_f64(), err);
    }

    // Catch redirect errors as success
    let result = match result {
        Err(e) if e.message == "__cflocation_redirect" || e.message == "__cfabort" => Ok(CfmlValue::Null),
        other => other,
    };

    if debug {
        if let Ok(ref value) = result {
            println!("Result: {:?}", value);
        }
    }

    // A transaction still open here was never closed by its block — `abort`, a
    // fatal error or a request timeout can all leave one behind. Roll it back
    // and drop the connection rather than handing it back to the pool
    // mid-transaction, where the next request picks it up dirty (GH #308).
    vm.rollback_open_transaction();

    // Take the response out of the VM via `mem::take` (leaving every field a
    // valid default) so the VM can then be DROPPED — releasing all of this
    // request's transient roots (page `variables`, request/thread scopes, call
    // frames, the per-request application-scope wrapper) in one go. Persistent
    // state already lives in `ServerState` (Arc-shared), so it survives the drop.
    let response = match result {
        Ok(_) => Ok(CfmlResponse {
            output: std::mem::take(&mut vm.output_buffer),
            response_headers: std::mem::take(&mut vm.response_headers),
            response_status: vm.response_status.take(),
            response_content_type: vm.response_content_type.take(),
            response_body: vm.response_body.take(),
            redirect_url: vm.redirect_url.take(),
            session_id: vm.session_id.take(),
            session_record_created: vm.session_record_created,
            session_cookie_policy: vm.session_cookie_policy.clone(),
            missing_template_handled: vm.missing_template_handled,
        }),
        Err(e) => Err(CfmlRunError {
            output: std::mem::take(&mut vm.output_buffer),
            message: format!("{}", e),
        }),
    };

    // Reclaim any reference cycles this request built (the serve-mode leak fix).
    // SKIP when a `cfthread` is still live — it shares scope Arcs across threads,
    // so `strong_count` reads would race. Dropping the VM first means the only
    // remaining strong refs into the request's graph are from persistent scopes
    // (live roots) or genuine garbage cycles, which `collect` then reclaims.
    if cfml_common::cycle_gc::is_armed() {
        // Pull out the join handles of any `cfthread` still GENUINELY RUNNING
        // (not merely lingering-but-finished). A finished thread has returned
        // from its body and dropped every Arc it held, so `strong_count` reads
        // are stable even though its handle still sits in `live_threads`
        // (cfthreads are fire-and-forget — nothing joins them). Only an
        // *executing* thread can race the count.
        //
        //  - No thread still running  → collect this request's cycles now.
        //  - Some thread still running → DEFER this request's log until those
        //    threads finish (never discard it — that would leak its cycles
        //    permanently). The deferred log is collected at a later request
        //    boundary or by the periodic sweep, once the threads complete.
        let running_joins: Vec<std::thread::JoinHandle<()>> = vm
            .live_threads
            .values_mut()
            .filter_map(|h| match &h.join {
                Some(j) if !j.is_finished() => h.join.take(),
                _ => None,
            })
            .collect();
        let any_running = !running_joins.is_empty();
        if std::env::var("RUSTCFML_GC_DEBUG").is_ok() {
            let (s, a, q, sc) = cfml_common::cycle_gc::log_type_breakdown();
            eprintln!(
                "[cycle_gc] request end: live_threads={} running={} log_len={:?} \
                 (structs={} arrays={} queries={} scopes={}) deferred_pending={}",
                vm.live_threads.len(),
                running_joins.len(),
                cfml_common::cycle_gc::log_len(),
                s, a, q, sc,
                cfml_common::cycle_gc::deferred_pending(),
            );
            if let Some(sites) = cfml_common::cycle_gc::drain_top_sites(25) {
                eprintln!("[cycle_gc] top sampled struct/array allocation sites this request:");
                for (site, n) in sites {
                    eprintln!("    {:>7}  {}", n, site);
                }
            }
        }
        drop(vm);
        if any_running {
            cfml_common::cycle_gc::defer_current_log(running_joins);
        } else {
            cfml_common::cycle_gc::collect();
            cfml_common::cycle_gc::disable_and_clear();
        }
        // Reclaim any previously-deferred logs whose threads have since finished.
        cfml_common::cycle_gc::collect_ready_deferred();
    }

    response
}

// ---------------------------------------------------------------------------
// Web server
// ---------------------------------------------------------------------------

struct AppState {
    doc_root: PathBuf,
    port: u16,
    debug: bool,
    server_state: ServerState,
    rewrite_rules: Vec<rewrite::RewriteRule>,
    vfs: Arc<dyn Vfs>,
    sandbox: bool,
    /// Cache of URL-path → resolved file. Only populated in production mode;
    /// keyed by the rewritten URL path (after rewrite rules have applied).
    resolved_file_cache: Arc<RwLock<HashMap<String, Option<ResolvedFile>>>>,
    /// Resolved RustCFML configuration. In production mode this is read once
    /// at startup; dev mode currently also reads once (live-reload lands in a
    /// later phase). Used for HTTP-block list and downstream wiring.
    cfconfig: Arc<RustCfmlConfig>,
}

#[allow(clippy::too_many_arguments)]
fn run_server(
    doc_root: &Path,
    port: u16,
    socket: Option<PathBuf>,
    debug: bool,
    single_threaded: bool,
    vfs: Arc<dyn Vfs>,
    sandbox: bool,
    production: bool,
    cfconfig: Arc<RustCfmlConfig>,
    profile: bool,
    memprofile_on: bool,
) {
    // Sampling heap profiler. Armed for the life of the server; dumps on
    // SIGUSR2 (no restart needed) and once more on graceful shutdown.
    #[cfg(all(feature = "memprofile", unix))]
    if memprofile_on {
        memprofile::arm("rustcfml-memprofile");
    }
    #[cfg(not(all(feature = "memprofile", unix)))]
    if memprofile_on {
        eprintln!("--memprofile requires a build with `--features memprofile` (Unix only)");
    }

    // Native sampling profiler (Phase 6) in serve mode. Sampling is process-wide
    // (a SIGPROF timer over all worker threads), so it captures aggregate CPU
    // across every request served — profile under load, then Ctrl+C to write the
    // flamegraph. The guard is a plain local held across `block_on` (a blocking
    // call, not an await), so there's no Send/await concern; the report is
    // written after the server shuts down gracefully, below.
    #[cfg(all(feature = "obs-pprof", unix))]
    let profiler = if profile {
        pprof_profile::start("rustcfml-profile")
    } else {
        None
    };
    #[cfg(not(all(feature = "obs-pprof", unix)))]
    if profile {
        eprintln!("--profile requires a build with `--features obs-pprof` (Unix only)");
    }

    // Arm the request-boundary cycle collector so a long-lived serve process
    // reclaims reference cycles instead of leaking them on every request. Opt
    // out with RUSTCFML_NO_CYCLE_GC=1.
    if std::env::var("RUSTCFML_NO_CYCLE_GC").map(|v| v != "0" && !v.is_empty()).unwrap_or(false) {
        log::info!("cycle collector disabled via RUSTCFML_NO_CYCLE_GC");
    } else {
        cfml_common::cycle_gc::arm();
    }

    let rt = if single_threaded {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(64 * 1024 * 1024) // 64MB — parity with the CLI main thread for deep ColdBox/WireBox autowiring
            .build()
            .unwrap()
    };
    rt.block_on(async_run_server(doc_root, port, socket, debug, single_threaded, vfs, sandbox, production, cfconfig));

    // Server has shut down gracefully (Ctrl+C) — write the flamegraph + pprof.
    #[cfg(all(feature = "obs-pprof", unix))]
    if let Some(session) = profiler {
        session.finish();
    }

    #[cfg(all(feature = "memprofile", unix))]
    if memprofile_on {
        memprofile::finish("rustcfml-memprofile");
    }
}

/// Known Lucee Memcached extension Java class names (both the old and new bundle).
const LUCEE_MEMCACHED_CLASSES: &[&str] = &[
    "org.lucee.extension.io.cache.memcache.memcacheraw",  // Lucee 5 / early 6
    "org.lucee.extension.cache.mc.memcachedcache",        // Lucee 6 current
];

/// Map a Lucee-style `class` field to a RustCFML provider string, or return
/// the `provider` field directly when no class is present.
fn resolve_provider(cache_cfg: &cfml_config::CacheCfg) -> &str {
    if !cache_cfg.provider.is_empty() {
        return &cache_cfg.provider;
    }
    let class_lc = cache_cfg.class.to_lowercase();
    if LUCEE_MEMCACHED_CLASSES.iter().any(|c| class_lc.as_str() == *c) {
        return "memcached";
    }
    ""
}

/// Parse a Memcached server list from either format:
/// - RustCFML `properties.servers` — `["host:port", ...]` JSON array
/// - Lucee `custom.servers` — `"host1:port host2:port"` space/comma-separated string
#[cfg(feature = "memcached")]
fn resolve_memcached_servers(cache_cfg: &cfml_config::CacheCfg) -> Vec<String> {
    if !cache_cfg.properties.servers.is_empty() {
        return cache_cfg.properties.servers.clone();
    }
    if let Some(raw) = cache_cfg.custom.get("servers") {
        return raw
            .split([' ', ','])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    Vec::new()
}

/// Build a `Discovery` strategy from the cluster cache properties.
///
/// Resolution order:
/// 1. `discovery.method` set explicitly → use that method.
/// 2. Legacy: `discovery.method` empty + `seeds` non-empty → "static".
/// 3. Otherwise → empty static (node starts solo).
#[cfg(feature = "cluster")]
fn build_discovery(
    props: &cfml_config::CacheProperties,
    listen_addr: &str,
) -> session::discovery::Discovery {
    use session::discovery::{Discovery, DnsDiscovery, MulticastDiscovery, StaticSeeds};

    // Default port for DNS/multicast: pull from listen_addr.
    let default_port: u16 = listen_addr
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .unwrap_or(7946);

    let method = props.discovery.method.trim().to_lowercase();
    let method = if method.is_empty() && !props.seeds.is_empty() {
        "static".to_string()
    } else if method.is_empty() {
        // Treat absent discovery + empty seeds as a solo-static node.
        "static".to_string()
    } else {
        method
    };

    match method.as_str() {
        "static" => {
            let seeds: Vec<String> = if !props.discovery.seeds.is_empty() {
                props.discovery.seeds.clone()
            } else {
                props.seeds.clone()
            };
            Discovery::Static(StaticSeeds::new(&seeds))
        }
        "dns" => {
            let name = props.discovery.name.clone();
            if name.is_empty() {
                eprintln!(
                    "[session/cluster] discovery.method=dns but discovery.name is empty — falling back to static seeds"
                );
                return Discovery::Static(StaticSeeds::new(&props.seeds));
            }
            let port = if props.discovery.port == 0 {
                default_port
            } else {
                props.discovery.port
            };
            Discovery::Dns(DnsDiscovery::new(name, port, props.discovery.interval_secs))
        }
        "multicast" => {
            let group = props.discovery.group.clone();
            let port = if props.discovery.port == 0 {
                default_port
            } else {
                props.discovery.port
            };
            // Advertise the listen address so peers can connect back. If
            // the bind is a wildcard, multicast announcements with that
            // address are useless to peers — warn the operator.
            let self_addr: std::net::SocketAddr =
                listen_addr.parse().unwrap_or_else(|_| {
                    "0.0.0.0:7946".parse().expect("hardcoded fallback addr is valid")
                });
            if self_addr.ip().is_unspecified() {
                eprintln!(
                    "[session/cluster] discovery.method=multicast but listenAddr is a wildcard ({}) — peers won't be able to dial back. Set listenAddr to a routable address.",
                    listen_addr
                );
            }
            match MulticastDiscovery::start(
                group,
                port,
                props.discovery.interval_secs,
                self_addr,
            ) {
                Ok(m) => Discovery::Multicast(m),
                Err(e) => {
                    eprintln!(
                        "[session/cluster] failed to start multicast discovery: {} — falling back to static seeds",
                        e
                    );
                    Discovery::Static(StaticSeeds::new(&props.seeds))
                }
            }
        }
        other => {
            eprintln!(
                "[session/cluster] unknown discovery.method='{}' — falling back to static seeds",
                other
            );
            Discovery::Static(StaticSeeds::new(&props.seeds))
        }
    }
}

/// Construct the session store from `.cfconfig.json` settings.
///
/// Resolution order: `sessionStorage` name in cfconfig → look up `caches`
/// entry → dispatch on `provider` (or Lucee `class`). Falls back to
/// `MemoryStore` if no config is present or the named cache is not found.
async fn build_session_store(
    cfconfig: &RustCfmlConfig,
    #[cfg_attr(not(feature = "cluster"), allow(unused_variables))]
    ws_registry: &Arc<cfml_vm::websocket::WebSocketRegistry>,
) -> Arc<dyn cfml_vm::session_store::SessionStore> {
    let storage_name = cfconfig.session_storage.trim();
    if storage_name.is_empty() || storage_name.eq_ignore_ascii_case("memory") {
        return Arc::new(cfml_vm::session_store::MemoryStore::new());
    }

    let cache_cfg = match cfconfig.caches.get(storage_name) {
        Some(c) => c,
        None => {
            // Lucee compat: `sessionStorage` may name a defined datasource
            // directly (no cache entry). That's the form Lucee `.cfconfig.json`
            // exports use for DB session storage.
            if cfconfig
                .datasources
                .keys()
                .any(|k| k.eq_ignore_ascii_case(storage_name))
            {
                println!(
                    "[session] Using datasource session store (datasource={}, table={})",
                    storage_name,
                    session::datasource::DEFAULT_TABLE
                );
                return Arc::new(session::datasource::DatasourceStore::new(
                    storage_name,
                    session::datasource::DEFAULT_TABLE,
                    storage_name,
                ));
            }
            eprintln!(
                "[session] sessionStorage cache '{}' not found in caches — using in-process memory store",
                storage_name
            );
            return Arc::new(cfml_vm::session_store::MemoryStore::new());
        }
    };

    // Warn if the cache is not flagged for storage (Lucee requires storage=true).
    // We emit a warning but do not refuse — RustCFML configs may omit the flag.
    if !cache_cfg.storage {
        eprintln!(
            "[session] Cache '{}' does not have storage=true — if this came from a Lucee \
             .cfconfig.json, add \"storage\": true to the cache definition",
            storage_name
        );
    }

    let provider = resolve_provider(cache_cfg).to_lowercase();

    match provider.as_str() {
        "memory" | "" => Arc::new(cfml_vm::session_store::MemoryStore::new()),

        "datasource" => {
            // `properties.datasource` names the backing datasource; fall back
            // to the cache name itself if omitted (a cache literally named
            // after the datasource). `properties.table` overrides the default.
            let ds = if !cache_cfg.properties.datasource.is_empty() {
                cache_cfg.properties.datasource.clone()
            } else {
                storage_name.to_string()
            };
            let table = if !cache_cfg.properties.table.is_empty() {
                cache_cfg.properties.table.clone()
            } else {
                session::datasource::DEFAULT_TABLE.to_string()
            };
            if !cfconfig
                .datasources
                .keys()
                .any(|k| k.eq_ignore_ascii_case(&ds))
            {
                eprintln!(
                    "[session] datasource provider references datasource '{}' which is not defined in cfconfig.datasources — using memory store",
                    ds
                );
                return Arc::new(cfml_vm::session_store::MemoryStore::new());
            }
            println!(
                "[session] Using datasource session store (datasource={}, table={})",
                ds, table
            );
            Arc::new(session::datasource::DatasourceStore::new(&ds, &table, &ds))
        }

        #[cfg(feature = "memcached")]
        "memcached" => {
            let servers = resolve_memcached_servers(cache_cfg);
            if servers.is_empty() {
                eprintln!("[session] Memcached provider configured but no servers listed — using memory store");
                return Arc::new(cfml_vm::session_store::MemoryStore::new());
            }
            let key_prefix = if !cache_cfg.properties.key_prefix.is_empty() {
                cache_cfg.properties.key_prefix.clone()
            } else {
                "rustcfml:sess:".to_string()
            };
            match session::memcached::MemcachedStore::new(&servers, &key_prefix) {
                Ok(store) => {
                    println!("[session] Using Memcached session store ({})", servers.join(", "));
                    Arc::new(store)
                }
                Err(e) => {
                    eprintln!("[session] Failed to connect to Memcached: {} — falling back to memory store", e);
                    Arc::new(cfml_vm::session_store::MemoryStore::new())
                }
            }
        }

        #[cfg(not(feature = "memcached"))]
        "memcached" => {
            eprintln!("[session] Memcached provider requested but binary was not compiled with --features memcached — using memory store");
            Arc::new(cfml_vm::session_store::MemoryStore::new())
        }

        #[cfg(feature = "cluster")]
        "cluster" => {
            let listen_addr = if !cache_cfg.properties.listen_addr.is_empty() {
                cache_cfg.properties.listen_addr.clone()
            } else {
                "0.0.0.0:7946".to_string()
            };
            let node_name = if !cache_cfg.properties.node_name.is_empty() {
                cache_cfg.properties.node_name.clone()
            } else if !cache_cfg.properties.advertise_addr.is_empty() {
                cache_cfg.properties.advertise_addr.clone()
            } else {
                format!("{}-{}", listen_addr, uuid::Uuid::new_v4().simple())
            };
            let discovery = build_discovery(&cache_cfg.properties, &listen_addr);
            let label = discovery.label();
            // Unify the WebSocket registry's node id with the cluster node name
            // BEFORE any connection: cross-node ids and `NodeGone` eviction key
            // off this match. Must happen before the registry mints conn ids.
            ws_registry.set_node_id(node_name.clone());
            let result = session::cluster::ClusterNode::start(
                &listen_addr,
                discovery,
                node_name.clone(),
                Some(ws_registry.clone()),
            )
            .await;
            match result {
                Ok(node) => {
                    println!(
                        "[session] Using cluster session store (listen={}, discovery={}); WebSocket fan-out clustered",
                        listen_addr, label
                    );
                    // Install the distributed fan-out adapter on the registry.
                    ws_registry.set_broker(node.ws_broker());
                    Arc::new(node.session_store())
                }
                Err(e) => {
                    eprintln!(
                        "[session] Failed to start cluster store on {}: {} — falling back to memory store",
                        listen_addr, e
                    );
                    Arc::new(cfml_vm::session_store::MemoryStore::new())
                }
            }
        }

        #[cfg(not(feature = "cluster"))]
        "cluster" => {
            eprintln!("[session] Cluster provider requested but binary was not compiled with --features cluster — using memory store");
            Arc::new(cfml_vm::session_store::MemoryStore::new())
        }

        other => {
            eprintln!("[session] Unknown session storage provider '{}' — using memory store", other);
            Arc::new(cfml_vm::session_store::MemoryStore::new())
        }
    }
}

/// Spawn the background session-expiry reaper. The reaper wakes on a timer,
/// drains every expired session out of the store (off the request path), and
/// queues an `onSessionEnd` delivery per drained session against its owning
/// application. The hook fires on the next request for that application
/// (cleanup-only delivery). A `reapIntervalSecs` of 0 disables the reaper
/// entirely — read-path exactness and native store TTL still apply.
fn spawn_session_reaper(server_state: cfml_vm::ServerState, cfconfig: &RustCfmlConfig) {
    let cfg = cfconfig.session.clone();
    if cfg.reap_interval_secs == 0 {
        println!("[session] background reaper disabled (reapIntervalSecs = 0)");
        return;
    }
    let interval = std::time::Duration::from_secs(cfg.reap_interval_secs);
    let batch_max = cfg.reap_batch_max;
    let adaptive = cfg.reap_adaptive;
    println!(
        "[session] background reaper enabled (interval={}s, adaptive={}, batchMax={})",
        cfg.reap_interval_secs, adaptive, batch_max
    );
    let sessions = server_state.sessions.clone();
    let pending_owner = server_state;

    tokio::spawn(async move {
        // First tick is a full interval away — don't reap immediately on boot.
        loop {
            // Decide how long to sleep. Adaptive: until the earliest known
            // expiry, capped at the configured tick (and floored at 1s so a
            // burst of already-expired sessions can't spin). Stores that can't
            // compute the next expiry return None and fall back to the tick.
            let delay = if adaptive {
                let now = now_unix_secs();
                match sessions.next_expiry(now) {
                    Some(next) => {
                        let secs = next.saturating_sub(now).clamp(1, cfg.reap_interval_secs);
                        std::time::Duration::from_secs(secs)
                    }
                    None => interval,
                }
            } else {
                interval
            };
            tokio::time::sleep(delay).await;

            let now = now_unix_secs();
            // `take_expired` may run blocking SQL (datasource store) — keep it
            // off the async worker threads.
            let sessions_for_blocking = sessions.clone();
            let drained =
                tokio::task::spawn_blocking(move || sessions_for_blocking.take_expired(now))
                    .await
                    .unwrap_or_default();

            if drained.is_empty() {
                continue;
            }
            let count = drained.len();
            let mut dropped_any = false;
            for (app_name, _id, vars) in drained {
                // app_name is "" for stores that don't record it (memcached /
                // KV); those never reach here because their take_expired is a
                // no-op, but guard anyway — an unkeyed hook can never be
                // delivered, so skip queuing it rather than stranding memory.
                if app_name.is_empty() {
                    continue;
                }
                if pending_owner.queue_session_end(&app_name, vars, batch_max) {
                    dropped_any = true;
                }
            }
            if dropped_any {
                log::warn!(
                    "[session] reaper dropped the oldest pending onSessionEnd for at least one \
                     application (reapBatchMax = {} reached — that application has not been \
                     requested recently)",
                    batch_max
                );
            }
            log::debug!("[session] reaper drained {} expired session(s)", count);
        }
    });
}

/// Current unix epoch seconds (wall clock).
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The serve-mode startup banner. Called ONLY after a listener has been bound,
/// so its appearance is proof the server is accepting — it previously printed
/// during setup and could be immediately followed by a fatal bind error.
///
/// Carries the binary's version so a running server can be identified at a
/// glance (which build is this? did my install actually take?) without having
/// to stop it and run `--version`.
fn print_ready_banner(what: &str, mode: &str, doc_root: &Path) {
    println!(
        "RustCFML server v{} {} ({})",
        env!("CARGO_PKG_VERSION"),
        what,
        mode
    );
    println!("Document root: {}", doc_root.display());
    println!("Press Ctrl+C to stop\n");
}

async fn async_run_server(
    doc_root: &Path,
    port: u16,
    socket: Option<PathBuf>,
    debug: bool,
    single_threaded: bool,
    vfs: Arc<dyn Vfs>,
    sandbox: bool,
    production: bool,
    cfconfig: Arc<RustCfmlConfig>,
) {
    let mut server_state = ServerState::with_config(production, cfconfig.clone());
    server_state.sessions = build_session_store(&cfconfig, &server_state.websocket).await;
    server_state.webroot = Some(
        fs::canonicalize(doc_root).unwrap_or_else(|_| doc_root.to_path_buf()),
    );

    // Sampling profiler (Phase 2 of the observability plan). When
    // `observability.profiler.enabled`, build the shared registry and spawn a
    // single watchdog thread that asks slow in-flight requests to sample their
    // own call stacks. Off by default — the field stays `None` and the VM never
    // installs a per-request handle, so the LineInfo hook remains a `None` branch.
    {
        let pcfg = &cfconfig.observability.profiler;
        if cfconfig.observability.enabled && pcfg.enabled {
            let hub = Arc::new(cfml_vm::profiler::ProfilerHub::new(
                pcfg.threshold_ms,
                pcfg.interval_ms,
                pcfg.max_samples,
            ));
            server_state.profiler = Some(hub.clone());
            let tick = std::time::Duration::from_millis(pcfg.watchdog_tick_ms.max(1));
            std::thread::Builder::new()
                .name("rustcfml-profiler-watchdog".into())
                .spawn(move || loop {
                    std::thread::sleep(tick);
                    hub.tick();
                })
                .expect("spawn profiler watchdog thread");
            println!(
                "Sampling profiler armed (threshold {}ms, interval {}ms, max {} samples)",
                pcfg.threshold_ms, pcfg.interval_ms, pcfg.max_samples
            );
        }
    }

    // OpenTelemetry (Phase 3): initialise the global tracer provider + OTLP
    // batch exporter and the Prometheus metric registry. Off by default; the
    // per-request span-building observer is installed only when this succeeds.
    #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
    {
        let ocfg = &cfconfig.observability.otel;
        if cfconfig.observability.enabled && ocfg.enabled {
            if otel::init(ocfg).is_some() {
                println!(
                    "OpenTelemetry enabled — traces → OTLP {} (sampleRatio {}), metrics → {}",
                    ocfg.endpoint, ocfg.sample_ratio, ocfg.metrics.prometheus_path
                );
            }
        }
    }

    // Populate the global datasource registry from cfconfig so cfquery /
    // queryExecute can resolve `datasource="myDSN"` lookups. Done once per
    // process; replaying with new values is idempotent for tests.
    populate_datasource_registry(&cfconfig);
    populate_default_mail_server(&cfconfig);
    cfml_stdlib::builtins::set_security_flags(cfml_stdlib::builtins::SecurityFlags {
        csrf_enabled: cfconfig.security.csrf_enabled,
        secure_json: cfconfig.security.secure_json,
        secure_json_prefix: cfconfig.security.secure_json_prefix.clone(),
    });

    // Load URL rewrite rules from cfconfig.urlRewriting.configFile (default
    // "urlrewrite.xml"). Skipped entirely if urlRewriting.enabled = false.
    let rewrite_rules = if !cfconfig.url_rewriting.enabled {
        Vec::new()
    } else {
        let cfg_path = if cfconfig.url_rewriting.config_file.is_empty() {
            "urlrewrite.xml".to_string()
        } else {
            cfconfig.url_rewriting.config_file.clone()
        };
        let rewrite_xml = if std::path::Path::new(&cfg_path).is_absolute() {
            PathBuf::from(&cfg_path)
        } else {
            doc_root.join(&cfg_path)
        };
        let rewrite_xml_str = rewrite_xml.to_string_lossy().to_string();
        if vfs.is_file(&rewrite_xml_str) {
            // Read through the VFS so embedded urlrewrite.xml in a
            // self-contained binary is honoured; reading the real filesystem
            // here would look for an absolute path that does not exist on the
            // deployment machine.
            let rules = match vfs.read_to_string(&rewrite_xml_str) {
                Ok(content) => rewrite::parse_urlrewrite_xml_content(&content),
                Err(e) => {
                    eprintln!("Warning: Could not read urlrewrite.xml: {}", e);
                    Vec::new()
                }
            };
            println!(
                "Loaded {} URL rewrite rule(s) from {}",
                rules.len(),
                rewrite_xml.display()
            );
            rules
        } else {
            Vec::new()
        }
    };

    let mode = if single_threaded { "single-threaded" } else { "multi-threaded" };
    // The banner is NOT printed here. It used to be, ~120 lines before the
    // listener was actually created, so a failed bind printed "RustCFML server
    // running on ..." and only then "Failed to start server: Address already in
    // use" — the success line arriving before the fatal error that contradicted
    // it. It is now emitted by `print_ready_banner` at each bind site, once the
    // listener exists, and reports the address actually bound rather than a
    // hardcoded 0.0.0.0 (which was wrong whenever `server.host` was set).
    let banner_root = fs::canonicalize(doc_root).unwrap_or_else(|_| doc_root.to_path_buf());

    // Spawn the background session reaper (serve mode only). It drains expired
    // session data off the request path on a timer and queues `onSessionEnd`
    // deliveries per application; the hooks themselves fire on the next request
    // for the owning application. Disabled when reapIntervalSecs = 0.
    spawn_session_reaper(server_state.clone(), &cfconfig);

    // Spawn the deferred cycle-GC sweep (serve mode only, when the collector is
    // armed). Request boundaries already drain deferred logs under load; this
    // timer guarantees a request that spawned a background thread is still
    // reclaimed once that thread finishes even if NO further request ever
    // arrives (an otherwise-idle server). Cheap: an uncontended lock + a length
    // check per tick when the queue is empty.
    if cfml_common::cycle_gc::is_armed() {
        tokio::spawn(async move {
            let tick = std::time::Duration::from_secs(2);
            loop {
                tokio::time::sleep(tick).await;
                tokio::task::spawn_blocking(cfml_common::cycle_gc::collect_ready_deferred)
                    .await
                    .ok();
            }
        });
    }

    // Bind address from cfconfig `server.host`. Captured here because
    // `cfconfig` is moved into `AppState` just below and `app_state` is itself
    // moved into the router before the listener is created.
    let bind_host = {
        let h = cfconfig.server.host.trim();
        if h.is_empty() { "0.0.0.0".to_string() } else { h.to_string() }
    };

    let app_state = Arc::new(AppState {
        doc_root: doc_root.to_path_buf(),
        port,
        debug,
        server_state,
        rewrite_rules,
        vfs,
        sandbox,
        resolved_file_cache: Arc::new(RwLock::new(HashMap::new())),
        cfconfig,
    });

    // socket.io transport (Phase 3): a tower layer that owns `/socket.io/` and
    // passes everything else through to the router below. Channel CFCs are
    // reached as socket.io namespaces (`/chat`) — the same CFCs the raw `/ws/`
    // route serves, sharing one registry.
    let socketio_layer = socketio::build_layer(app_state.clone());

    let app = axum::Router::new()
        // Raw-WebSocket upgrade for channel CFCs under <docroot>/websockets/.
        // Everything else falls through to the normal request handler.
        .route("/ws/{channel}", axum::routing::get(websocket::ws_handler))
        .fallback(handle_request)
        .layer(socketio_layer)
        .with_state(app_state);

    match socket {
        // --socket: bind a Unix domain socket instead of a TCP port.
        Some(sock_path) => {
            #[cfg(unix)]
            {
                // Remove any stale socket file left over from a previous run;
                // bind(2) fails with EADDRINUSE if the path already exists.
                if sock_path.exists() {
                    if let Err(e) = std::fs::remove_file(&sock_path) {
                        eprintln!("Failed to remove stale socket {}: {}", sock_path.display(), e);
                        exit(1);
                    }
                }
                let listener = tokio::net::UnixListener::bind(&sock_path).unwrap_or_else(|e| {
                    eprintln!("Failed to bind Unix socket {}: {}", sock_path.display(), e);
                    exit(1);
                });
                print_ready_banner(
                    &format!("listening on Unix socket: {}", sock_path.display()),
                    mode,
                    &banner_root,
                );
                // Unix-socket peers have no IP address, so the
                // ConnectInfo<SocketAddr> extractor in `handle_request` would
                // fail at runtime under `into_make_service()`. Inject a synthetic
                // loopback ConnectInfo extension on every request so the shared
                // handler works unchanged.
                let app = app.layer(axum::middleware::map_request(
                    |mut req: axum::extract::Request| async move {
                        req.extensions_mut().insert(axum::extract::ConnectInfo(
                            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
                        ));
                        req
                    },
                ));
                // Graceful shutdown: on Ctrl+C, stop accepting and then remove the
                // socket file so the next start binds cleanly.
                let cleanup_path = sock_path.clone();
                axum::serve(listener, app.into_make_service())
                    .with_graceful_shutdown(async move {
                        let _ = tokio::signal::ctrl_c().await;
                    })
                    .await
                    .unwrap();
                #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
                otel::shutdown();
                let _ = std::fs::remove_file(&cleanup_path);
            }
            #[cfg(not(unix))]
            {
                let _ = &sock_path;
                eprintln!("Unix domain sockets are not supported on this platform");
                exit(1);
            }
        }
        // Default: TCP listener on the configured port.
        None => {
            // `server.host` from cfconfig. This key existed but was never read —
            // the listener hardcoded "0.0.0.0", so a config asking to bind
            // loopback-only was silently ignored and the server was reachable
            // from every interface regardless. The default is "0.0.0.0" (what
            // the engine has always done); set the key to restrict it.
            let host = bind_host;
            let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await.unwrap_or_else(|e| {
                eprintln!("Failed to start server on {}:{}: {}", host, port, e);
                exit(1);
            });
            // Report the address the OS actually gave us — with `--port 0` that
            // is the assigned ephemeral port, not the literal 0 that was asked for.
            let bound = listener
                .local_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| format!("{host}:{port}"));
            print_ready_banner(&format!("running on http://{bound}"), mode, &banner_root);
            // Disable Nagle's algorithm on accepted connections. For a request/response
            // HTTP server, Nagle adds latency by holding small writes, and can stall on
            // the classic Nagle + delayed-ACK interaction. axum::serve does not set this
            // by default, so opt in via tap_io. (Go net/http, nginx, Node all default on.)
            use axum::serve::ListenerExt;
            let listener = listener.tap_io(|tcp_stream| {
                let _ = tcp_stream.set_nodelay(true);
            });
            // Graceful shutdown on Ctrl+C so `main` returns normally. This lets
            // a held resource (e.g. the DHAT Profiler under `--features
            // dhat-heap`) run its Drop and flush its dump instead of being
            // killed mid-flight by the signal.
            axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .with_graceful_shutdown(async move {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .unwrap();
            #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
            otel::shutdown();
        }
    }
}

/// Render one call-tree node (and its children) as JSON for the profiler
/// admin endpoint.
fn profiler_node_json(node: &cfml_vm::profiler::CallNode, total: f64) -> serde_json::Value {
    serde_json::json!({
        "function": node.function,
        "template": node.template,
        "line": node.line,
        "self": node.self_count,
        "total": node.total_count,
        "selfPercent": (node.self_count as f64 / total) * 100.0,
        "totalPercent": (node.total_count as f64 / total) * 100.0,
        "children": node.children.iter().map(|c| profiler_node_json(c, total)).collect::<Vec<_>>(),
    })
}

/// The `/__rustcfml/profiler` admin endpoint. Returns `None` when the profiler
/// is off (so the request falls through to the normal 404 path); otherwise a
/// JSON document of the recent profiled requests, newest first.
fn profiler_endpoint(state: &Arc<AppState>) -> Option<axum::response::Response> {
    use axum::response::IntoResponse;
    let hub = state.server_state.profiler.as_ref()?;
    let profiles: Vec<serde_json::Value> = hub
        .recent_profiles()
        .iter()
        .map(|p| {
            let total = p.tree.total_count.max(1) as f64;
            serde_json::json!({
                "id": p.id,
                "route": p.route,
                "sampleCount": p.sample_count,
                "tree": profiler_node_json(&p.tree, total),
            })
        })
        .collect();
    let body = serde_json::json!({ "profiles": profiles });
    Some(
        (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string()),
        )
            .into_response(),
    )
}

/// Request entry point. Wraps [`handle_request_inner`] so that whichever of its
/// many exits is taken, the request's buffered `<cflog>` lines reach disk before
/// the response goes out — the durability bound we document for log buffering
/// (at most one request's lines can be lost to a hard crash).
async fn handle_request(
    state: axum::extract::State<Arc<AppState>>,
    addr: axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let response = handle_request_inner(state, addr, req).await;
    logging::flush_all();
    response
}

/// Did this body-read failure come from the size cap, or from the transport?
///
/// `axum::body::to_bytes` wraps both in the same opaque `axum::Error`, so the
/// only honest way to tell them apart is to walk the source chain looking for
/// `http_body_util::LengthLimitError`. Anything else (client aborted mid-POST,
/// truncated body, connection reset) is NOT a size problem and must not be
/// reported as one.
fn body_read_error_is_length_limit(err: &axum::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = source {
        if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() {
            return true;
        }
        source = e.source();
    }
    false
}

async fn handle_request_inner(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.to_string();
    let remote_addr = addr.ip().to_string();
    let url = parts.uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/").to_string();

    // Extract headers as Vec<(String, String)>
    let mut headers: Vec<(String, String)> = parts.headers.iter()
        .map(|(name, value)| (name.as_str().to_string(), value.to_str().unwrap_or("").to_string()))
        .collect();

    // Read body bytes. `server.maxRequestBodySize` from cfconfig caps the
    // allowed size (`0` = unlimited).
    let body_limit = match state.cfconfig.server.max_request_body_size {
        0 => usize::MAX,
        n => n as usize,
    };
    // An over-limit body MUST fail loudly with 413. This previously used
    // `.unwrap_or_default()`, which silently substituted an EMPTY body: the
    // request then ran with no form scope at all (not just a missing file), so
    // a too-large upload surfaced as a baffling downstream "variable is
    // undefined". Preside's asset upload hit exactly that.
    //
    // `to_bytes` fails for two unrelated reasons and they MUST NOT be conflated:
    // an actual `LengthLimitError`, or a transport error (client aborted, body
    // truncated, connection reset). Reporting the latter as 413 sends the
    // uploader chasing a size limit it never came close to — we shipped exactly
    // that, logging "exceeds maxRequestBodySize (209715200 bytes)" for a request
    // whose declared content-length was 23282.
    let body_bytes = match axum::body::to_bytes(body, body_limit).await {
        Ok(bytes) => bytes,
        Err(e) => {
            let declared = parts
                .headers
                .get(axum::http::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            if body_read_error_is_length_limit(&e) {
                // Note the limit can trip before the whole body is read, so the
                // declared content-length is logged for context rather than a
                // byte count we measured.
                log::error!(
                    "413 {method} {url}: request body exceeds server.maxRequestBodySize ({} bytes); \
                     declared content-length {declared}: {e}",
                    state.cfconfig.server.max_request_body_size
                );
                return axum::response::Response::builder()
                    .status(413)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(axum::body::Body::from(format!(
                        "Request body too large. The server limit is {} bytes \
                         (server.maxRequestBodySize in cfconfig; 0 = unlimited).",
                        state.cfconfig.server.max_request_body_size
                    )))
                    .unwrap();
            }
            log::error!(
                "400 {method} {url}: could not read the request body \
                 (declared content-length {declared}, server.maxRequestBodySize {} bytes \
                 was NOT reached): {e}",
                state.cfconfig.server.max_request_body_size
            );
            return axum::response::Response::builder()
                .status(400)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(axum::body::Body::from(format!(
                    "Could not read the request body: {e}"
                )))
                .unwrap();
        }
    };

    let debug = state.debug;

    log::debug!("{} {}", method, url);

    // Split URL into path and query string
    let (raw_path, original_qs) = match url.find('?') {
        Some(pos) => (&url[..pos], &url[pos + 1..]),
        None => (url.as_str(), ""),
    };

    // Sampling-profiler admin endpoint (Phase 2). Returns the recent profiled
    // requests (slow requests the watchdog sampled) as JSON. Only served when
    // the profiler is actually armed, so it 404s exactly like any other unknown
    // path when the feature is off.
    if raw_path == "/__rustcfml/profiler" {
        if let Some(resp) = profiler_endpoint(&state) {
            return resp;
        }
    }

    // OpenTelemetry RED metrics scrape endpoint (Phase 3). Prometheus text
    // exposition; 404s (falls through) when metrics are off.
    #[cfg(all(feature = "obs-otel", not(target_arch = "wasm32")))]
    if raw_path == state.cfconfig.observability.otel.metrics.prometheus_path {
        if let Some(text) = otel::render_metrics() {
            use axum::response::IntoResponse;
            return (
                [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                text,
            )
                .into_response();
        }
    }

    // Block direct access to config and meta files. 404 (not 403) so the
    // existence of the file is not confirmed to a probing client. Always
    // applies regardless of any rewrite rules.
    if is_blocked_path(raw_path, &state.cfconfig) {
        log::debug!("  -> 404 (blocked path)");
        return axum::response::Response::builder()
            .status(404)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(axum::body::Body::from("Not found"))
            .unwrap();
    }

    // Apply URL rewrite rules before file resolution
    let mut path = raw_path.to_string();
    let mut query_string = original_qs.to_string();
    // When a Forward rewrite maps the request onto a front controller (e.g.
    // Preside/ColdBox `/admin/` → `/index.cfm`), the original URI must still be
    // discoverable by the app's SES router. On Lucee behind Tuckey that arrives
    // via the servlet forward attributes / `X-Original-URL`; RustCFML has no
    // servlet layer, so we surface the original URI as an `X-Original-URL`
    // request header (Preside's `pathInfoProvider` reads it first). Without this
    // the forwarded path is lost — `cgi.path_info` becomes "/" — and every deep
    // URL (the whole admin) mis-routes to the site homepage.
    let mut forward_original_uri: Option<String> = None;
    if !state.rewrite_rules.is_empty() {
        let mut header_map = HashMap::new();
        for (name, value) in &headers {
            header_map.insert(name.clone(), value.clone());
        }

        if let Some(result) = rewrite::apply_rewrite_rules(&state.rewrite_rules, &path, &method, state.port, &header_map, original_qs, &remote_addr) {
            match result.rewrite_type {
                rewrite::RewriteType::PermanentRedirect => {
                    log::debug!("  -> 301 redirect to {}", result.new_path);
                    return axum::response::Response::builder()
                        .status(301)
                        .header("Location", &result.new_path)
                        .body(axum::body::Body::empty())
                        .unwrap();
                }
                rewrite::RewriteType::Redirect => {
                    log::debug!("  -> 302 redirect to {}", result.new_path);
                    return axum::response::Response::builder()
                        .status(302)
                        .header("Location", &result.new_path)
                        .body(axum::body::Body::empty())
                        .unwrap();
                }
                rewrite::RewriteType::Forward => {
                    if result.new_path != path {
                        log::debug!("  -> rewrite to {}", result.new_path);
                    }
                    // Split rewritten path from its query string
                    if let Some(qpos) = result.new_path.find('?') {
                        let rewritten_qs = &result.new_path[qpos + 1..];
                        // Merge: rewritten QS params, then original QS params
                        if !rewritten_qs.is_empty() && !query_string.is_empty() {
                            query_string = format!("{}&{}", rewritten_qs, query_string);
                        } else if !rewritten_qs.is_empty() {
                            query_string = rewritten_qs.to_string();
                        }
                        path = result.new_path[..qpos].to_string();
                    } else {
                        path = result.new_path;
                    }
                    // A forward that changed the executed path onto a front
                    // controller: remember the original request URI (path +
                    // original query string) so the app's SES router can recover
                    // it. Only when the path actually changed — a no-op forward
                    // (path unchanged) leaves normal path_info handling intact.
                    if path != raw_path {
                        forward_original_uri = Some(if original_qs.is_empty() {
                            raw_path.to_string()
                        } else {
                            format!("{}?{}", raw_path, original_qs)
                        });
                    }
                }
            }
        }
    }

    // Surface the pre-rewrite URI as `X-Original-URL` (see note above). Replace
    // any client-supplied value so it cannot spoof the router's view of the URL.
    if let Some(ref original_uri) = forward_original_uri {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("x-original-url"));
        headers.push(("X-Original-URL".to_string(), original_uri.clone()));
    }

    // Resolve file path from URL (memoized in production mode)
    let welcome_files = &state.cfconfig.server.welcome_files;
    let cfml_exts = &state.cfconfig.server.cfml_extensions;
    let resolved = if state.server_state.production_mode {
        let cached = state.resolved_file_cache.read().unwrap().get(&path).cloned();
        match cached {
            Some(hit) => hit,
            None => {
                let r = resolve_file(&state.doc_root, &path, state.vfs.as_ref(), welcome_files, cfml_exts);
                state
                    .resolved_file_cache
                    .write()
                    .unwrap()
                    .insert(path.clone(), r.clone());
                r
            }
        }
    } else {
        resolve_file(&state.doc_root, &path, state.vfs.as_ref(), welcome_files, cfml_exts)
    };

    // Front-controller fallback: an unresolved URL is routed to a configured
    // template (instead of 404), with the original path exposed in the URL
    // scope and the original query string preserved.
    let fallback = &state.cfconfig.server.fallback;
    let resolved = if resolved.is_none() && !fallback.template.is_empty() {
        match resolve_file(
            &state.doc_root,
            &fallback.template,
            state.vfs.as_ref(),
            welcome_files,
            cfml_exts,
        ) {
            Some(rf) => {
                // Inject the original path as url.<routeParam>, ahead of the
                // original params (the template reads it via the URL scope).
                let route_param = if fallback.route_param.is_empty() {
                    "route"
                } else {
                    fallback.route_param.as_str()
                };
                let route_pair = format!("{}={}", route_param, query_encode(&path));
                query_string = if query_string.is_empty() {
                    route_pair
                } else {
                    format!("{}&{}", route_pair, query_string)
                };
                log::debug!("  -> front-controller fallback to {}", fallback.template);
                Some(rf)
            }
            None => None,
        }
    } else {
        resolved
    };

    // CFML extensions to dispatch through the interpreter. `.cfc` files are
    // never served as static even if absent from cfml_extensions — that would
    // expose source. Likewise `.cfm` stays in the default list.
    let is_cfml_file = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| cfml_exts.iter().any(|ext| ext.eq_ignore_ascii_case(e)))
    };

    match resolved {
        Some(ref rf) if is_cfml_file(&rf.file_path) => {
            // Execute .cfm file — in non-debug serve mode the bytecode cache reads the
            // file itself, so we pass an empty source and skip the redundant read.
            let source = if debug {
                match fs::read_to_string(&rf.file_path) {
                    Ok(s) => s,
                    Err(e) => {
                        return axum::response::Response::builder()
                            .status(500)
                            .header("Content-Type", "text/html; charset=utf-8")
                            .body(axum::body::Body::from(format!(
                                "<html><body><h1>500 Internal Server Error</h1><p>Error reading file: {}</p></body></html>",
                                html_escape(&e.to_string())
                            )))
                            .unwrap();
                    }
                }
            } else {
                String::new()
            };

            // Build web scopes using resolved script_name and path_info
            let (extra_globals, http_request_data) = build_web_scopes(
                &method, &headers, &body_bytes, &rf.script_name, &rf.path_info, &query_string, state.port, &remote_addr,
            );

            let file_path = rf.file_path.to_string_lossy().to_string();
            let server_state = state.server_state.clone();
            let vfs = state.vfs.clone();
            let sandbox = state.sandbox;

            // Extract the session ID from the incoming CFID cookie, if any.
            // Under the lazy-session default we do NOT pre-mint an id here:
            // the VM mints one only when code actually writes to session, so a
            // read-only/never-touched request hands out no cookie.
            let existing_sid: Option<String> = {
                let cookie_header = headers.iter()
                    .find(|(n, _)| n.to_lowercase() == "cookie")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                cookie_header.split(';')
                    .find_map(|c| {
                        let c = c.trim();
                        if c.starts_with("CFID=") {
                            Some(c[5..].to_string())
                        } else {
                            None
                        }
                    })
            };
            let session_id_clone = existing_sid.clone();

            // The server is HTTP-only and meant to sit behind a TLS-terminating
            // proxy, so a request is "secure" iff the proxy told us so via
            // `X-Forwarded-Proto: https`. Drives the auto-`Secure` cookie default.
            let conn_is_secure = headers.iter().any(|(n, v)| {
                n.eq_ignore_ascii_case("x-forwarded-proto") && v.eq_ignore_ascii_case("https")
            });

            let result = tokio::task::spawn_blocking(move || {
                compile_and_run_with_session(
                    &source,
                    debug,
                    Some(file_path),
                    extra_globals,
                    Some(&server_state),
                    Some(http_request_data),
                    session_id_clone,
                    vfs,
                    sandbox,
                )
            }).await.unwrap();

            match result {
                Ok(response) => build_success_response(response, existing_sid.as_ref(), conn_is_secure),
                Err(e) => {
                    render_error_response(&state, &e)
                }
            }
        }
        Some(ref rf) => {
            // Serve static file (via VFS for embedded support)
            match state.vfs.read(&rf.file_path.to_string_lossy()) {
                Ok(data) => {
                    let ct = content_type_for(&rf.file_path);
                    axum::response::Response::builder()
                        .status(200)
                        .header("Content-Type", ct)
                        .body(axum::body::Body::from(data))
                        .unwrap()
                }
                Err(_) => {
                    axum::response::Response::builder()
                        .status(500)
                        .header("Content-Type", "text/html; charset=utf-8")
                        .body(axum::body::Body::from(
                            "<html><body><h1>500 Internal Server Error</h1><p>Error reading file</p></body></html>"
                        ))
                        .unwrap()
                }
            }
        }
        None => {
            // A missing template that maps to the CFML engine (.cfm/.cfc) gets a
            // chance to be handled by Application.cfc onMissingTemplate before the
            // default 404. Non-CFML 404s (images, .html, directory requests) bypass
            // the engine entirely — they never reach a handler in ColdFusion/Lucee.
            if is_cfml_file(Path::new(&path)) {
                let relative = path.trim_start_matches('/');
                let would_be_path = state.doc_root.join(relative).to_string_lossy().to_string();
                // target_page is web-root-relative, as ColdFusion passes it.
                let target_page = if path.starts_with('/') { path.clone() } else { format!("/{}", path) };

                let (extra_globals, http_request_data) = build_web_scopes(
                    &method, &headers, &body_bytes, &target_page, "", &query_string, state.port, &remote_addr,
                );

                let existing_sid: Option<String> = {
                    let cookie_header = headers.iter()
                        .find(|(n, _)| n.to_lowercase() == "cookie")
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    cookie_header.split(';').find_map(|c| {
                        let c = c.trim();
                        c.strip_prefix("CFID=").map(|v| v.to_string())
                    })
                };
                let conn_is_secure = headers.iter().any(|(n, v)| {
                    n.eq_ignore_ascii_case("x-forwarded-proto") && v.eq_ignore_ascii_case("https")
                });

                let server_state = state.server_state.clone();
                let vfs = state.vfs.clone();
                let sandbox = state.sandbox;
                let session_id_clone = existing_sid.clone();
                let result = tokio::task::spawn_blocking(move || {
                    run_missing_template(
                        would_be_path,
                        target_page,
                        extra_globals,
                        Some(&server_state),
                        Some(http_request_data),
                        session_id_clone,
                        vfs,
                        sandbox,
                    )
                }).await.unwrap();

                match result {
                    Ok(response) if response.missing_template_handled => {
                        return build_success_response(response, existing_sid.as_ref(), conn_is_secure);
                    }
                    // Unhandled (no handler, or it returned false) → default 404.
                    Ok(_) => {}
                    // A throw inside onMissingTemplate (with no onError) surfaces as
                    // a normal 500 error page, like any other uncaught CFML error.
                    Err(e) => {
                        return render_error_response(&state, &e);
                    }
                }
            }

            axum::response::Response::builder()
                .status(404)
                .header("Content-Type", "text/html; charset=utf-8")
                .body(axum::body::Body::from(format!(
                    "<html><body><h1>404 Not Found</h1><p>The requested URL {} was not found.</p></body></html>",
                    html_escape(&path)
                )))
                .unwrap()
        }
    }
}

/// Build the HTTP response for a successful CFML execution: honour a
/// `cflocation` redirect, resolve singleton headers + content-type, pick the
/// `cfheader` status (default 200), emit the body, and attach the session
/// `CFID` cookie when one was minted or rotated this request. Shared by the
/// normal target-page path and the onMissingTemplate path.
fn build_success_response(
    response: CfmlResponse,
    existing_sid: Option<&String>,
    conn_is_secure: bool,
) -> axum::response::Response<axum::body::Body> {
    // Check for redirect
    if let Some(ref redirect_url) = response.redirect_url {
        let status_code = response.response_status
            .as_ref()
            .map(|(c, _)| *c)
            .unwrap_or(302);
        let mut builder = axum::response::Response::builder()
            .status(status_code)
            .header("Location", redirect_url.as_str());
        for (name, value) in &response.response_headers {
            if name.to_lowercase() != "location" {
                builder = builder.header(name.as_str(), value.as_str());
            }
        }
        // Carry the session cookie through a redirect too, so a
        // write-then-cflocation (the POST/redirect/GET pattern)
        // hands the new CFID to the browser.
        if let Some(ref sid) = response.session_id {
            if response.session_record_created || Some(sid) != existing_sid {
                builder = builder.header(
                    "Set-Cookie",
                    response.session_cookie_policy.render("CFID", sid, conn_is_secure),
                );
            }
        }
        return builder.body(axum::body::Body::empty()).unwrap();
    }

    // Resolve the singleton Content-Type and de-duplicate other
    // singleton headers so a cfheader(name="Content-Type") (or
    // Content-Length/Location) replaces rather than appends to
    // the engine default (issue #148).
    let (content_type, emit_headers) = cfml_vm::web::resolve_response_headers(
        response.response_content_type.as_deref(),
        &response.response_headers,
    );

    // Determine body. A binary cfcontent payload (CfmlValue::Binary) must be
    // emitted as raw bytes; routing it through as_string() would substitute the
    // diagnostic placeholder text and corrupt the response.
    let body = if let Some(ref body_override) = response.response_body {
        match body_override {
            CfmlValue::Binary(bytes) => axum::body::Body::from(bytes.clone()),
            other => axum::body::Body::from(other.as_string()),
        }
    } else {
        axum::body::Body::from(response.output)
    };

    // Determine status code
    let status_code = response.response_status
        .as_ref()
        .map(|(c, _)| *c)
        .unwrap_or(200);

    let mut builder = axum::response::Response::builder()
        .status(status_code)
        .header("Content-Type", content_type.as_str());

    for (name, value) in &emit_headers {
        builder = builder.header(name.as_str(), value.as_str());
    }

    // Set session cookie only when a record was minted this
    // request (lazy default: a read-only request writes nothing
    // and gets no cookie) or the id changed (sessionRotate).
    if let Some(ref sid) = response.session_id {
        if response.session_record_created || Some(sid) != existing_sid {
            builder = builder.header(
                "Set-Cookie",
                response.session_cookie_policy.render("CFID", sid, conn_is_secure),
            );
        }
    }

    builder.body(body).unwrap()
}

use cfml_vm::web::{ResolvedFile, resolve_file};

/// Build CGI, URL, and Form scopes from extracted HTTP request data.
///
/// Thin shim over `cfml_vm::web::build_web_scopes` retained for callsite
/// stability inside the CLI serve handler.
fn build_web_scopes(
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
    script_name: &str,
    path_info: &str,
    query_string: &str,
    port: u16,
    remote_addr: &str,
) -> (ValueMap, CfmlValue) {
    cfml_vm::web::build_web_scopes(
        method, headers, body, script_name, path_info, query_string, port, remote_addr,
    )
}


fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Percent-encode a value for safe placement in a query-string pair. Encodes
/// the characters that would otherwise be reinterpreted by the query parser
/// (`&`, `=`, `+`, `%`, space) while leaving path-friendly chars like `/` and
/// `.` intact, so it round-trips through `parse_query_string`'s `url_decode`.
fn query_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'&' | b'=' | b'+' | b'%' | b' ' | b'#' | b'?' => {
                out.push_str(&format!("%{:02X}", b));
            }
            // Non-ASCII and control bytes: percent-encode each byte so the
            // value survives the decoder and stays valid UTF-8.
            _ if b < 0x20 || b >= 0x7F => out.push_str(&format!("%{:02X}", b)),
            _ => out.push(b as char),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the response body for an unhandled CFML execution error, honoring
/// the `debugging.*` section of cfconfig:
///   * `enabled`        — when false, the underlying error message is hidden
///                        from the client; full detail still hits the server log.
///   * `errorStatusCode` — when false, response status is 200 (handy when
///                        a front-end takes over error rendering).
///   * `errorTemplate`   — when set, the file is rendered as a fresh CFML
///                        request with the error available as request._error.
///                        Render failure falls back to the inline page.
fn render_error_response(state: &Arc<AppState>, e: &CfmlRunError) -> axum::response::Response {
    let dbg = &state.cfconfig.debugging;
    let status: u16 = if dbg.error_status_code { 500 } else { 200 };

    // Always log full detail server-side regardless of the public surface.
    log::error!("CFML error: {}", e.message);

    // errorTemplate path: best effort. If the template isn't readable or
    // execution itself errors, fall through to the inline render.
    if !dbg.error_template.is_empty() {
        let template_path = if std::path::Path::new(&dbg.error_template).is_absolute() {
            PathBuf::from(&dbg.error_template)
        } else {
            state.doc_root.join(&dbg.error_template)
        };
        if let Ok(source) = fs::read_to_string(&template_path) {
            let mut extra = ValueMap::default();
            let mut err_struct = ValueMap::default();
            err_struct.insert("message".to_string(), CfmlValue::string(e.message.clone()));
            err_struct.insert("output".to_string(), CfmlValue::string(e.output.clone()));
            let mut req = ValueMap::default();
            req.insert("_error".to_string(), CfmlValue::strukt(err_struct));
            extra.insert("request".to_string(), CfmlValue::strukt(req));
            if let Ok(resp) = compile_and_run(
                &source,
                state.debug,
                Some(template_path.to_string_lossy().to_string()),
                extra,
                Some(&state.server_state),
                None,
                None,
                state.vfs.clone(),
                state.sandbox,
                None,
                true,
                true,
            ) {
                let (content_type, emit_headers) = cfml_vm::web::resolve_response_headers(
                    resp.response_content_type.as_deref(),
                    &resp.response_headers,
                );
                let mut builder = axum::response::Response::builder()
                    .status(status)
                    .header("Content-Type", content_type.as_str());
                for (k, v) in &emit_headers {
                    builder = builder.header(k, v);
                }
                return builder
                    .body(axum::body::Body::from(resp.output))
                    .unwrap();
            }
        }
        log::warn!(
            "debugging.errorTemplate '{}' could not be rendered; falling back",
            dbg.error_template
        );
    }

    // Inline render.
    let mut body = String::new();
    if !e.output.is_empty() {
        body.push_str(&e.output);
    }
    if dbg.enabled {
        body.push_str(&format!(
            "<html><body><h1>{} Internal Server Error</h1><pre>{}</pre></body></html>",
            status,
            html_escape(&e.message)
        ));
    } else {
        body.push_str(&format!(
            "<html><body><h1>{} Internal Server Error</h1><p>The server encountered an error processing your request.</p></body></html>",
            status,
        ));
    }
    axum::response::Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// Resolve cfconfig for a `--build`-produced binary at runtime. Resolution
/// order, first match wins:
///   1. `.cfconfig.json` in the same directory as the running binary
///      (operator override — no rebuild needed)
///   2. `.cfconfig.json` embedded in the VFS at build time (read via the
///      `vfs` arg using the embedded base dir)
///   3. Compiled-in defaults
fn load_embedded_cfconfig(vfs: &dyn Vfs, base_dir: &str) -> RustCfmlConfig {
    // 1. External next to the binary
    if let Some(dir) = resolve::exe_dir() {
        let external = dir.join(".cfconfig.json");
        if external.is_file() {
            match RustCfmlConfig::from_file(&external) {
                Ok(c) => {
                    log::info!("loaded external .cfconfig.json from {}", external.display());
                    return c;
                }
                Err(e) => log::warn!("failed to load external .cfconfig.json: {}", e),
            }
        }
    }
    // 2. Embedded copy
    let embedded_path = format!("{}/.cfconfig.json", base_dir);
    if vfs.is_file(&embedded_path) {
        if let Ok(bytes) = vfs.read(&embedded_path) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                match RustCfmlConfig::from_str(text) {
                    Ok(c) => {
                        log::info!("loaded embedded .cfconfig.json from VFS");
                        return c;
                    }
                    Err(e) => log::warn!("failed to parse embedded .cfconfig.json: {}", e),
                }
            }
        }
    }
    // 3. Defaults
    RustCfmlConfig::default()
}

/// Push every cfconfig datasource into the cfml-stdlib global registry so
/// cfquery / queryExecute can resolve `datasource="myDSN"` to a connection
/// URL. Drivers that don't compile in (e.g. mssql without the feature) are
/// still registered — they'll fail later at query time with a clearer
/// "driver not available" error from the stdlib than from a silent miss.
fn populate_datasource_registry(cfg: &RustCfmlConfig) {
    // Relative SQLite paths anchor to the directory of the .cfconfig.json that
    // declared them (not the process cwd), so a configured `database: "app.db"`
    // lands next to the config predictably instead of in the launch directory.
    let base_dir = cfg
        .source_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(std::path::Path::to_path_buf);
    for (name, ds) in cfg.datasources.iter() {
        if let Some(url) = ds.connection_url_anchored(base_dir.as_deref()) {
            cfml_stdlib::builtins::register_datasource(name, url.clone());
            cfml_stdlib::builtins::register_datasource_timeout(&url, ds.connection_timeout);
            if ds.default {
                cfml_stdlib::builtins::set_default_datasource(url);
            }
        } else {
            log::warn!(
                "cfconfig datasource '{}': unrecognised driver '{}'; skipping",
                name,
                ds.driver
            );
        }
    }
}

/// First mailServers entry from cfconfig becomes the process-wide default
/// for cfmail when its tag attributes omit `server`. Empty list leaves the
/// stdlib unconfigured — cfmail will keep raising the existing
/// "no SMTP Server defined" error.
fn populate_default_mail_server(cfg: &RustCfmlConfig) {
    if let Some(m) = cfg.mail_servers.first() {
        cfml_stdlib::builtins::set_default_mail_server(
            cfml_stdlib::builtins::DefaultMailServer {
                server: m.smtp.clone(),
                port: m.port,
                username: m.username.clone(),
                password: m.password.clone(),
                tls: m.tls,
                ssl: m.ssl,
                timeout: m.timeout,
            },
        );
    }
}

/// Return true if the URL path points at a file the web server must not serve
/// directly. Covers `.cfconfig*` / `.env` / `*.lex` unconditionally, then
/// applies the glob patterns from `security.blockedPaths`.
fn is_blocked_path(url_path: &str, cfconfig: &RustCfmlConfig) -> bool {
    let basename = url_path.rsplit('/').next().unwrap_or(url_path);
    if cfml_config::resolve::is_protected_filename(basename) {
        return true;
    }
    let url_lower = url_path.to_ascii_lowercase();
    let base_lower = basename.to_ascii_lowercase();
    for pat in &cfconfig.security.blocked_paths {
        let pat_lower = pat.to_ascii_lowercase();
        if glob_matches(&pat_lower, &base_lower) || glob_matches(&pat_lower, &url_lower) {
            return true;
        }
    }
    false
}

/// Minimal `*` glob matcher (no `?`, no character classes). Sufficient for
/// the simple patterns used in `security.blockedPaths` (e.g. `*.cfm.bak`,
/// `Application.cfc`, `*.config.cfm`).
fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<&str> = pattern.split('*').collect();
    if p.len() == 1 {
        return pattern == text;
    }
    let mut cursor = 0;
    if !p[0].is_empty() {
        if !text.starts_with(p[0]) {
            return false;
        }
        cursor = p[0].len();
    }
    for (idx, segment) in p.iter().enumerate().skip(1) {
        if segment.is_empty() {
            continue;
        }
        if idx == p.len() - 1 {
            // Last segment must match the tail.
            if cursor > text.len() {
                return false;
            }
            return text[cursor..].ends_with(segment);
        }
        match text[cursor..].find(segment) {
            Some(pos) => cursor += pos + segment.len(),
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod cfconfig_block_tests {
    use super::*;

    #[test]
    fn glob_star_extension() {
        assert!(glob_matches("*.cfm.bak", "foo.cfm.bak"));
        assert!(!glob_matches("*.cfm.bak", "foo.cfm"));
    }

    #[test]
    fn glob_exact_name() {
        assert!(glob_matches("application.cfc", "application.cfc"));
        assert!(!glob_matches("application.cfc", "myapp.cfc"));
    }

    #[test]
    fn glob_internal_wildcard() {
        assert!(glob_matches("foo*bar", "foo123bar"));
        assert!(glob_matches("foo*bar", "foobar"));
        assert!(!glob_matches("foo*bar", "foo123"));
    }

    #[test]
    fn block_protects_cfconfig() {
        let cfg = RustCfmlConfig::default();
        assert!(is_blocked_path("/.cfconfig.json", &cfg));
        assert!(is_blocked_path("/sub/.cfconfig.json", &cfg));
        assert!(is_blocked_path("/.CFConfig.json", &cfg));
        assert!(is_blocked_path("/.env", &cfg));
        assert!(!is_blocked_path("/index.cfm", &cfg));
    }

    #[test]
    fn block_honors_default_patterns() {
        let cfg = RustCfmlConfig::default();
        assert!(is_blocked_path("/Application.cfc", &cfg));
        assert!(is_blocked_path("/sub/foo.cfm.bak", &cfg));
    }
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

/// Compile a single REPL line through the full pipeline (tag preprocess → lex →
/// parse → bytecode), returning a runnable program or a human-readable error.
fn compile_repl_line(source: &str, debug: bool) -> Result<cfml_codegen::BytecodeProgram, String> {
    let source = if tag_parser::has_cfml_tags(source) {
        tag_parser::tags_to_script_checked(source)?
    } else {
        source.to_string()
    };
    let tokens = lexer::tokenize(source.clone());
    if debug {
        eprintln!("=== TOKENS ===");
        for (i, tok) in tokens.iter().enumerate() {
            eprintln!("{:3}: {:?}", i, tok.token);
        }
    }
    let ast = CfmlParser::new(source)
        .parse()
        .map_err(|e| format!("Parse Error [line {}, col {}]: {}", e.line, e.column, e.message))?;
    Ok(CfmlCompiler::new().compile(ast))
}

fn run_repl(debug: bool) {
    println!("RustCFML REPL v{}", env!("CARGO_PKG_VERSION"));
    println!("Type 'exit' or 'quit' to exit\n");

    // A single, persistent VM lives for the whole session so that variables and
    // function declarations carry across lines (the page/`variables` scope is
    // `vm.globals`, which `repl_eval` preserves). Start from an empty program
    // and swap each line's compiled `__main__` in via `repl_eval`.
    let empty = match compile_repl_line("", false) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("REPL init failed: {}", e);
            return;
        }
    };
    let mut vm = CfmlVirtualMachine::new(empty);
    vm.vfs = vfs::real_fs();
    register_vm_runtime(&mut vm);
    // CFML guarantees url/cgi/form always exist.
    vm.globals.entry("url".to_string()).or_insert_with(|| CfmlValue::strukt(ValueMap::default()));
    vm.globals.entry("cgi".to_string()).or_insert_with(empty_magic_cgi);
    vm.globals.entry("form".to_string()).or_insert_with(|| CfmlValue::strukt(ValueMap::default()));

    loop {
        print!("cfml> ");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();

        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap() == 0 {
            println!();
            break;
        }

        let line = line.trim();
        if line == "exit" || line == "quit" {
            break;
        }

        if line.is_empty() {
            continue;
        }

        let program = match compile_repl_line(line, debug) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{}", e);
                continue;
            }
        };

        let result = vm.repl_eval(program);
        vm.finalize_html_injections();
        if !vm.output_buffer.is_empty() {
            print!("{}", vm.output_buffer);
            // Ensure the next prompt starts on its own line.
            if !vm.output_buffer.ends_with('\n') {
                println!();
            }
        }
        // An error prints but never terminates the session — a typo at the
        // prompt must not kill the REPL (it used to `exit(1)`).
        if let Err(e) = result {
            eprintln!("{}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Self-contained binary: build & run
// ---------------------------------------------------------------------------

/// Build a self-contained binary by embedding all files from `app_dir` into a
/// copy of the current rustcfml executable.
///
/// Two paths:
/// - **Bundling path (default).** App contains no `native/` directory. We
///   read the running `rustcfml` exe, strip any existing archive, and append
///   the app's VFS archive. Toolchain-free.
/// - **Cocktail path.** App contains `native/<mod>/Cargo.toml` for one or more
///   Rust modules. We generate a Cargo workspace that depends on
///   `rustcfml-cli` plus each user module, synthesise a `main.rs` that calls
///   `run_with_registrar`, shell out to `cargo build --release`, and then
///   append the VFS archive to the freshly-built binary. Requires `cargo`
///   on PATH.
fn build_self_contained(app_dir: &str, output: &str, mode: &str, entry: &str) {
    use std::collections::HashMap;

    let app_path = PathBuf::from(app_dir);
    if !app_path.is_dir() {
        eprintln!("Error: '{}' is not a directory", app_dir);
        exit(1);
    }

    let app_path = fs::canonicalize(&app_path).unwrap_or(app_path);
    println!("Embedding app from: {}", app_path.display());
    println!("Mode: {}, Entry: {}", mode, entry);

    let native_modules = discover_native_modules(&app_path);
    if !native_modules.is_empty() {
        println!(
            "Detected {} native module(s): {}",
            native_modules.len(),
            native_modules
                .iter()
                .map(|m| m.crate_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Walk directory and collect all files
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    collect_files(&app_path, &app_path, &mut files);

    if files.is_empty() {
        eprintln!("Error: No files found in '{}'", app_dir);
        exit(1);
    }

    // Validate entry point exists for CLI mode
    if mode == "cli" {
        let entry_lower = entry.to_lowercase();
        if !files.keys().any(|k| k.to_lowercase() == entry_lower) {
            eprintln!("Error: Entry point '{}' not found in '{}'", entry, app_dir);
            eprintln!("Available files: {}", files.keys().cloned().collect::<Vec<_>>().join(", "));
            exit(1);
        }
    }

    // Embed metadata: mode and entry point
    let meta = format!("mode={}\nentry={}", mode, entry);
    files.insert("__rustcfml_meta__".to_string(), meta.into_bytes());

    let total_size: usize = files.values().map(|v| v.len()).sum();
    println!("Collected {} files ({:.1} KB)", files.len() - 1, total_size as f64 / 1024.0);

    // Determine base binary. With native modules we have to shell out to
    // cargo build a fresh binary that wires in the registrar calls; without
    // them we can reuse the currently-running rustcfml exe (toolchain-free).
    let base_binary: Vec<u8> = if native_modules.is_empty() {
        let exe_path = std::env::current_exe().expect("Cannot determine current executable path");
        let base_binary = fs::read(&exe_path).expect("Cannot read current executable");
        strip_existing_archive(&base_binary).to_vec()
    } else {
        match build_cocktail_binary(&app_path, &native_modules) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("Error: {}", e);
                exit(1);
            }
        }
    };

    // On macOS, strip the code signature from the base binary before
    // appending. Apple Silicon requires signed binaries, so we'll re-sign
    // after writing. The cocktail path produces a fresh binary that hasn't
    // been signed yet — strip is a no-op then but cheap.
    #[cfg(target_os = "macos")]
    let base_binary = strip_macos_signature(base_binary);
    #[cfg(not(target_os = "macos"))]
    let base_binary = base_binary;

    // Create self-contained binary
    let output_data = vfs::create_self_contained_binary(&base_binary, &files);

    // Write output
    let output_path = PathBuf::from(output);
    fs::write(&output_path, &output_data).expect("Failed to write output binary");

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&output_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&output_path, perms).unwrap();
    }

    // On macOS (especially Apple Silicon), binaries must be code-signed.
    // The base binary's signature was stripped before appending; now re-sign.
    #[cfg(target_os = "macos")]
    {
        let out = output_path.to_str().unwrap_or("");
        let status = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-", "--no-strict", out])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Ok(s) = status {
            if !s.success() {
                eprintln!("Warning: Failed to re-sign binary (codesign). The binary may not run on macOS.");
            }
        }
    }

    // Verify the archive is extractable from the final binary
    let final_data = fs::read(&output_path).expect("Cannot read output binary for verification");
    if vfs::extract_archive_from_bytes(&final_data).is_none() {
        eprintln!("Error: Archive verification failed — the embedded archive is not readable.");
        eprintln!("This may be caused by code signing. Try running: codesign --remove-signature {}", output_path.display());
        exit(1);
    }

    println!("Built: {} ({:.1} MB)", output_path.display(), final_data.len() as f64 / (1024.0 * 1024.0));
    println!("Run with: ./{}", output_path.display());
}

// ---------------------------------------------------------------------------
// Cocktail build: native modules + cargo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct NativeModule {
    /// Absolute path to the module crate (the directory containing its
    /// `Cargo.toml`).
    crate_path: PathBuf,
    /// Cargo package name as declared in the module's `Cargo.toml`. Used as
    /// the path-dep key in the generated workspace.
    crate_name: String,
    /// Rust import name (`crate_name` with dashes converted to underscores).
    /// Used in the generated `main.rs`.
    import_name: String,
}

/// Walk `app_dir/native/` and return each immediate subdirectory that
/// contains a `Cargo.toml`. Returns empty when the project has no native
/// modules — that's the toolchain-free bundling path.
fn discover_native_modules(app_dir: &Path) -> Vec<NativeModule> {
    let native_dir = app_dir.join("native");
    if !native_dir.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let entries = match fs::read_dir(&native_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let crate_path = entry.path();
        if !crate_path.is_dir() {
            continue;
        }
        let manifest = crate_path.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let manifest_text = match fs::read_to_string(&manifest) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Trivial parse: find `name = "..."` in `[package]`.
        let crate_name = manifest_text
            .lines()
            .skip_while(|l| !l.trim_start().starts_with("[package]"))
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('['))
            .find_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("name")?.trim_start();
                let rest = rest.strip_prefix('=')?.trim();
                let rest = rest.strip_prefix('"')?;
                let end = rest.find('"')?;
                Some(rest[..end].to_string())
            });
        let crate_name = match crate_name {
            Some(n) => n,
            None => {
                eprintln!(
                    "Warning: skipping native module at {} — could not read [package].name from Cargo.toml",
                    crate_path.display()
                );
                continue;
            }
        };
        let import_name = crate_name.replace('-', "_");
        out.push(NativeModule {
            crate_path,
            crate_name,
            import_name,
        });
    }
    out.sort_by(|a, b| a.crate_name.cmp(&b.crate_name));
    out
}

/// Locate the RustCFML source checkout so we can use `rustcfml-cli` as a
/// path dependency in the generated workspace.
///
/// Resolution order:
/// 1. `RUSTCFML_SOURCE` env var pointing at a workspace root.
/// 2. `CARGO_MANIFEST_DIR` baked in at compile time — two levels up from
///    `crates/cli` is the workspace root. Works during development.
///
/// Returns `None` when neither path exists on disk. Module-using `--build`
/// requires a checkout (or a published rustcfml-cli on crates.io, which
/// doesn't exist yet).
fn rustcfml_source_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSTCFML_SOURCE") {
        let pb = PathBuf::from(p);
        if pb.join("Cargo.toml").is_file() {
            return Some(pb);
        }
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = PathBuf::from(manifest_dir).parent()?.parent()?.to_path_buf();
    if root.join("Cargo.toml").is_file() {
        return Some(root);
    }
    None
}

/// Detect whether `cargo` is callable on PATH. Returns the version string on
/// success.
fn probe_cargo() -> Result<String, String> {
    let out = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to invoke cargo: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "cargo exited with status {:?}",
            out.status.code()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Format a filesystem path as a TOML literal string (single-quoted, no
/// escape processing). Works for any platform path that doesn't contain a
/// literal single-quote character — paths with `'` would conflict with TOML
/// literal-string syntax and are rejected with a clear error rather than
/// silently producing broken output.
fn toml_path_literal(path: &Path) -> Result<String, String> {
    let s = path.to_string_lossy();
    if s.contains('\'') {
        return Err(format!(
            "path contains a single quote, which conflicts with TOML \
             literal-string syntax used in the generated Cargo.toml: {}",
            s
        ));
    }
    Ok(format!("'{}'", s))
}

/// Generate the cocktail workspace, run `cargo build --release`, and return
/// the bytes of the resulting binary (ready to have a VFS archive appended).
fn build_cocktail_binary(
    app_path: &Path,
    modules: &[NativeModule],
) -> Result<Vec<u8>, String> {
    let cargo_version = probe_cargo().map_err(|e| {
        format!(
            "This project references native Rust modules under `native/`. \
             Building requires a Rust toolchain (cargo/rustc) on PATH — install \
             from https://rustup.rs and re-run. ({})",
            e
        )
    })?;
    println!("Using: {}", cargo_version);

    let source_root = rustcfml_source_root().ok_or_else(|| {
        "Could not locate the RustCFML source checkout. Set RUSTCFML_SOURCE \
         to the absolute path of your RustCFML repo, then re-run."
            .to_string()
    })?;

    // Build dir: app/.rustcfml-cocktail/. Deterministic, inspectable, lives
    // alongside the app so cargo's incremental cache survives between runs.
    let build_dir = app_path.join(".rustcfml-cocktail");
    let _ = fs::create_dir_all(build_dir.join("src"));

    // Write Cargo.toml. Path values use TOML literal strings (single
    // quotes) so Windows backslashes don't need escaping — and so we
    // don't depend on Rust's Debug formatter producing valid TOML.
    let mut deps = String::new();
    deps.push_str(&format!(
        "rustcfml-cli = {{ path = {} }}\n",
        toml_path_literal(&source_root.join("crates").join("cli"))?
    ));
    for m in modules {
        deps.push_str(&format!(
            "{} = {{ path = {} }}\n",
            m.crate_name,
            toml_path_literal(&m.crate_path)?
        ));
    }
    let manifest = format!(
        r#"# Auto-generated by `rustcfml --build`. Do not edit by hand.
[workspace]

[package]
name = "rustcfml-cocktail-build"
version = "0.0.0"
edition = "2021"
publish = false

[[bin]]
name = "rustcfml-cocktail"
path = "src/main.rs"

[dependencies]
{deps}
"#,
        deps = deps
    );
    fs::write(build_dir.join("Cargo.toml"), manifest)
        .map_err(|e| format!("write Cargo.toml: {}", e))?;

    // Synthesise main.rs: chain each module's register(vm) inside
    // run_with_registrar.
    let imports: String = modules
        .iter()
        .map(|m| format!("use {} as {};\n", m.import_name, m.import_name))
        .collect();
    let calls: String = modules
        .iter()
        .map(|m| format!("        {}::register(vm);\n", m.import_name))
        .collect();
    let main_src = format!(
        r#"// Auto-generated by `rustcfml --build`. Do not edit by hand.
{imports}
fn main() {{
    rustcfml_cli::run_with_registrar(|vm| {{
{calls}    }});
}}
"#,
        imports = imports,
        calls = calls
    );
    fs::write(build_dir.join("src").join("main.rs"), main_src)
        .map_err(|e| format!("write main.rs: {}", e))?;

    // Run cargo build --release inside the build dir. We forward cargo's
    // stdout/stderr to the terminal (the default for .status()) so the user
    // sees compile errors live rather than having them buffered.
    println!("Mixing cocktail (cargo build --release)…");
    let status = std::process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(&build_dir)
        .status()
        .map_err(|e| format!("invoke cargo build: {}", e))?;
    if !status.success() {
        // Surface a multi-line hint pointing at the build dir so users can
        // iterate on their module without going through `rustcfml --build`
        // each time (which re-generates the workspace).
        eprintln!();
        eprintln!("─── Cocktail build failed ───");
        eprintln!("To iterate on the error above without re-running `rustcfml --build`:");
        eprintln!("    cd {}", build_dir.display());
        eprintln!("    cargo build --release");
        eprintln!("The generated workspace there path-deps directly on your native modules,");
        eprintln!("so edits to native/<crate>/src/*.rs pick up incrementally.");
        eprintln!();
        return Err(format!(
            "cargo build failed (exit {:?}). Build dir: {}",
            status.code(),
            build_dir.display()
        ));
    }

    let bin_path = build_dir
        .join("target")
        .join("release")
        .join(if cfg!(windows) {
            "rustcfml-cocktail.exe"
        } else {
            "rustcfml-cocktail"
        });
    fs::read(&bin_path).map_err(|e| {
        format!("read cocktail binary at {}: {}", bin_path.display(), e)
    })
}

/// Recursively collect files from a directory into a HashMap.
/// Keys are relative paths with forward slashes.
fn collect_files(base: &Path, dir: &Path, files: &mut std::collections::HashMap<String, Vec<u8>>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Warning: Cannot read directory {}: {}", dir.display(), e);
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories, common non-app dirs, and — when at
            // the root — the `native/` directory itself (Rust source has
            // been compiled into the binary already; no need to also ship
            // it in the embedded archive).
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" || name == ".git" {
                continue;
            }
            if name == "native" && path.parent() == Some(base) {
                continue;
            }
            collect_files(base, &path, files);
        } else if path.is_file() {
            let relative = path.strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            match fs::read(&path) {
                Ok(data) => {
                    files.insert(relative, data);
                }
                Err(e) => {
                    eprintln!("Warning: Cannot read {}: {}", path.display(), e);
                }
            }
        }
    }
}

/// Strip macOS code signature from binary data using codesign CLI.
/// This is needed because appending data to a signed Mach-O invalidates the
/// signature, and codesign won't re-sign a binary with a stale signature.
#[cfg(target_os = "macos")]
fn strip_macos_signature(data: Vec<u8>) -> Vec<u8> {
    let tmp = std::env::temp_dir().join("rustcfml_strip_sig");
    if fs::write(&tmp, &data).is_ok() {
        let status = std::process::Command::new("codesign")
            .args(["--remove-signature", tmp.to_str().unwrap_or("")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Ok(s) = status {
            if s.success() {
                if let Ok(stripped) = fs::read(&tmp) {
                    let _ = fs::remove_file(&tmp);
                    return stripped;
                }
            }
        }
        let _ = fs::remove_file(&tmp);
    }
    data
}

/// Strip an existing embedded archive from a binary (if present).
fn strip_existing_archive(data: &[u8]) -> &[u8] {
    let len = data.len();
    if len < vfs::ARCHIVE_MAGIC.len() + 8 {
        return data;
    }
    let magic_start = len - vfs::ARCHIVE_MAGIC.len();
    if &data[magic_start..] != vfs::ARCHIVE_MAGIC.as_slice() {
        return data;
    }
    let len_start = magic_start - 8;
    let archive_len = u64::from_le_bytes(data[len_start..len_start + 8].try_into().unwrap()) as usize;
    let archive_start = len_start - archive_len;
    &data[..archive_start]
}

/// Parse metadata from the embedded archive.
fn parse_embedded_meta(files: &std::collections::HashMap<String, Vec<u8>>) -> (String, String) {
    let meta = files.get("__rustcfml_meta__")
        .map(|data| String::from_utf8_lossy(data).to_string())
        .unwrap_or_default();
    let mut mode = "serve".to_string();
    let mut entry = "main.cfm".to_string();
    for line in meta.lines() {
        if let Some(val) = line.strip_prefix("mode=") {
            mode = val.to_string();
        } else if let Some(val) = line.strip_prefix("entry=") {
            entry = val.to_string();
        }
    }
    (mode, entry)
}

/// Run the embedded app (self-contained binary mode).
/// Supports both "serve" (web server) and "cli" (command-line) modes.
fn run_embedded_app(mut files: std::collections::HashMap<String, Vec<u8>>) {
    use cfml_common::vfs::{EmbeddedFs, FallbackFs, RealFs};

    let (mode, entry) = parse_embedded_meta(&files);

    // Remove metadata file from the archive so it's not visible to CFML code
    files.remove("__rustcfml_meta__");
    let file_count = files.len();

    // Determine base dir: use CWD as the virtual base
    let base_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    // FallbackFs: try embedded files first, then real filesystem.
    // This allows embedded apps to load external files (e.g. modules on disk).
    // Sandbox mode is determined later per-invocation; default to non-sandboxed
    // here — run_embedded_cli/serve will set it based on --sandbox flag.
    let vfs: Arc<dyn Vfs> = Arc::new(FallbackFs {
        embedded: EmbeddedFs::new(files, base_dir.clone()),
        real: RealFs,
        sandbox: false,
    });

    if mode == "cli" {
        run_embedded_cli(vfs, &base_dir, &entry, file_count);
    } else {
        run_embedded_serve(vfs, &base_dir, file_count);
    }
}

/// Run embedded app in CLI mode: execute entry point with command-line args.
fn run_embedded_cli(vfs: Arc<dyn Vfs>, base_dir: &str, entry: &str, file_count: usize) {
    let cli_args: Vec<String> = std::env::args().collect();

    let mut sandbox = false;
    // Check for --help / --version / --sandbox
    for arg in &cli_args[1..] {
        match arg.as_str() {
            "--version" => {
                println!("Built with RustCFML v{} ({} embedded files)", env!("CARGO_PKG_VERSION"), file_count);
                exit(0);
            }
            // A `--build` binary redistributes the same statically linked
            // dependencies, so it carries the same attribution obligation.
            "--licenses" => {
                print_licenses();
                exit(0);
            }
            "--sandbox" => { sandbox = true; }
            _ => {}
        }
    }

    // Build the entry point path and read source from VFS
    let entry_path = format!("{}/{}", base_dir, entry);
    let source = match vfs.read_to_string(&entry_path) {
        Ok(s) => s,
        Err(_) => {
            // Try just the entry name (relative)
            match vfs.read_to_string(entry) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: Cannot read entry point '{}': {}", entry, e);
                    exit(1);
                }
            }
        }
    };

    // Parse CLI args into the "cli" scope (ordered struct).
    // Works like CFML's arguments scope:
    //   --name value  → cli.name = "value"   (named)
    //   --flag        → cli.flag = true       (boolean flag)
    //   positional    → cli[1], cli[2], ...   (1-based numeric keys)
    let mut cli_scope = ValueMap::default();
    let mut positional_idx: usize = 1;
    let mut i = 1;
    while i < cli_args.len() {
        let arg = &cli_args[i];
        if arg.starts_with("--") {
            let raw = arg.trim_start_matches('-');
            // Handle --key=value syntax
            if let Some(eq_pos) = raw.find('=') {
                let key = raw[..eq_pos].to_lowercase();
                let value = raw[eq_pos + 1..].to_string();
                cli_scope.insert(key, CfmlValue::string(value));
                i += 1;
            } else {
                let key = raw.to_lowercase();
                if i + 1 < cli_args.len() && !cli_args[i + 1].starts_with("--") {
                    cli_scope.insert(key, CfmlValue::string(cli_args[i + 1].clone()));
                    i += 2;
                } else {
                    cli_scope.insert(key, CfmlValue::Bool(true));
                    i += 1;
                }
            }
        } else if arg.starts_with("-") && !arg.starts_with("--") {
            let raw = arg.trim_start_matches('-');
            if let Some(eq_pos) = raw.find('=') {
                let key = raw[..eq_pos].to_lowercase();
                let value = raw[eq_pos + 1..].to_string();
                cli_scope.insert(key, CfmlValue::string(value));
                i += 1;
            } else {
                let key = raw.to_lowercase();
                if i + 1 < cli_args.len() && !cli_args[i + 1].starts_with("-") {
                    cli_scope.insert(key, CfmlValue::string(cli_args[i + 1].clone()));
                    i += 2;
                } else {
                    cli_scope.insert(key, CfmlValue::Bool(true));
                    i += 1;
                }
            }
        } else {
            // Positional: 1-based numeric key like CFML arguments scope
            cli_scope.insert(positional_idx.to_string(), CfmlValue::string(arg.clone()));
            positional_idx += 1;
            i += 1;
        }
    }

    let mut extra_globals = ValueMap::default();
    extra_globals.insert("cli".to_string(), CfmlValue::strukt(cli_scope));

    // Execute
    match compile_and_run(&source, false, Some(entry_path), extra_globals, None, None, None, vfs, sandbox, None, false, false) {
        Ok(response) => {
            if !response.output.is_empty() {
                print!("{}", response.output);
            }
        }
        Err(e) => {
            if !e.output.is_empty() {
                print!("{}", e.output);
            }
            eprintln!("{}", e.message);
            exit(1);
        }
    }
}

/// Run embedded app in serve mode with start/stop/foreground support.
fn run_embedded_serve(vfs: Arc<dyn Vfs>, base_dir: &str, file_count: usize) {
    let cli_args: Vec<String> = std::env::args().collect();

    // Parse args
    let mut port: u16 = 8500;
    let mut socket: Option<PathBuf> = None;
    let mut single_threaded = false;
    let mut sandbox = false;
    let mut command = ""; // "", "start", "stop", "status"
    let mut i = 1;
    while i < cli_args.len() {
        match cli_args[i].as_str() {
            "--port" if i + 1 < cli_args.len() => {
                port = cli_args[i + 1].parse().unwrap_or(8500);
                i += 2;
            }
            // --socket [PATH]: bind a Unix socket; bare flag uses the default path.
            "--socket" => {
                if i + 1 < cli_args.len() && !cli_args[i + 1].starts_with('-') {
                    socket = Some(PathBuf::from(&cli_args[i + 1]));
                    i += 2;
                } else {
                    socket = Some(PathBuf::from("/run/rustcfml.sock"));
                    i += 1;
                }
            }
            "--single-threaded" => {
                single_threaded = true;
                i += 1;
            }
            "--sandbox" => {
                sandbox = true;
                i += 1;
            }
            "--version" => {
                println!("RustCFML v{} (self-contained, {} files)", env!("CARGO_PKG_VERSION"), file_count);
                exit(0);
            }
            // Same obligation as the CLI-mode embedded binary above.
            "--licenses" => {
                print_licenses();
                exit(0);
            }
            "start" | "stop" | "status" => {
                command = match cli_args[i].as_str() {
                    "start" => "start",
                    "stop" => "stop",
                    "status" => "status",
                    _ => "",
                };
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "app".to_string());
    let pid_file = format!("/tmp/{}.pid", exe_name);

    match command {
        "stop" => {
            embedded_stop(&pid_file);
        }
        "status" => {
            embedded_status(&pid_file);
        }
        "start" => {
            embedded_start(&pid_file, port, file_count);
            // After daemonizing, the child process continues here
            if sandbox { println!("Sandbox mode: host filesystem access disabled"); }
            println!("RustCFML self-contained app ({} embedded files)", file_count);
            let doc_root = PathBuf::from(base_dir);
            // Sandbox-mode binaries serve exclusively from the embedded
            // archive (immutable at runtime), so production-mode caches are
            // always safe and beneficial. Non-sandbox binaries can read from
            // the host FS, so respect the explicit env-var opt-in.
            let production = sandbox
                || std::env::var("RUSTCFML_PRODUCTION").as_deref() == Ok("1");
            // Resolve cfconfig for an embedded binary: external file next to
            // the exe wins so operators can tweak settings without rebuilding;
            // otherwise read the copy embedded into the VFS at build time;
            // otherwise defaults.
            let cfconfig = Arc::new(load_embedded_cfconfig(vfs.as_ref(), base_dir));
            populate_datasource_registry(&cfconfig);
            populate_default_mail_server(&cfconfig);
            cfml_stdlib::builtins::set_security_flags(cfml_stdlib::builtins::SecurityFlags {
                csrf_enabled: cfconfig.security.csrf_enabled,
                secure_json: cfconfig.security.secure_json,
                secure_json_prefix: cfconfig.security.secure_json_prefix.clone(),
            });
            run_server(&doc_root, port, socket, false, single_threaded, vfs, sandbox, production, cfconfig, false, false);
        }
        _ => {
            // Foreground mode (default: just run)
            if sandbox { println!("Sandbox mode: host filesystem access disabled"); }
            println!("RustCFML self-contained app ({} embedded files)", file_count);
            let doc_root = PathBuf::from(base_dir);
            // Sandbox-mode binaries serve exclusively from the embedded
            // archive (immutable at runtime), so production-mode caches are
            // always safe and beneficial. Non-sandbox binaries can read from
            // the host FS, so respect the explicit env-var opt-in.
            let production = sandbox
                || std::env::var("RUSTCFML_PRODUCTION").as_deref() == Ok("1");
            // Resolve cfconfig for an embedded binary: external file next to
            // the exe wins so operators can tweak settings without rebuilding;
            // otherwise read the copy embedded into the VFS at build time;
            // otherwise defaults.
            let cfconfig = Arc::new(load_embedded_cfconfig(vfs.as_ref(), base_dir));
            populate_datasource_registry(&cfconfig);
            populate_default_mail_server(&cfconfig);
            cfml_stdlib::builtins::set_security_flags(cfml_stdlib::builtins::SecurityFlags {
                csrf_enabled: cfconfig.security.csrf_enabled,
                secure_json: cfconfig.security.secure_json,
                secure_json_prefix: cfconfig.security.secure_json_prefix.clone(),
            });
            run_server(&doc_root, port, socket, false, single_threaded, vfs, sandbox, production, cfconfig, false, false);
        }
    }
}

/// Daemonize: fork to background and write PID file.
#[cfg(unix)]
fn embedded_start(pid_file: &str, port: u16, file_count: usize) {
    use std::io::Write;

    // Check if already running
    if let Ok(pid_str) = fs::read_to_string(pid_file) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            // Check if process is alive
            if unsafe { libc::kill(pid, 0) } == 0 {
                eprintln!("Already running (PID {})", pid);
                exit(1);
            }
        }
    }

    // Fork
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            eprintln!("Failed to fork");
            exit(1);
        }
        0 => {
            // Child process — continue to run the server
            // Create new session
            unsafe { libc::setsid() };

            // Write PID file
            let child_pid = std::process::id();
            let mut f = std::fs::File::create(pid_file).expect("Cannot create PID file");
            write!(f, "{}", child_pid).expect("Cannot write PID file");

            // Redirect stdout/stderr to log file
            let log_path = pid_file.replace(".pid", ".log");
            if let Ok(log_file) = std::fs::File::create(&log_path) {
                use std::os::unix::io::AsRawFd;
                let fd = log_file.as_raw_fd();
                unsafe {
                    libc::dup2(fd, 1); // stdout
                    libc::dup2(fd, 2); // stderr
                }
            }
            // Child continues to the server startup code
        }
        _ => {
            // Parent process — report and exit
            println!("Started in background (PID {})", pid);
            println!("Listening on http://127.0.0.1:{} ({} embedded files)", port, file_count);
            println!("Stop with: {} stop", std::env::args().next().unwrap_or_default());
            exit(0);
        }
    }
}

#[cfg(not(unix))]
fn embedded_start(pid_file: &str, _port: u16, _file_count: usize) {
    // On non-Unix, just write PID and run in foreground
    let pid = std::process::id();
    let _ = fs::write(pid_file, format!("{}", pid));
}

/// Stop a daemonized instance by reading its PID file.
#[cfg(unix)]
fn embedded_stop(pid_file: &str) {
    match fs::read_to_string(pid_file) {
        Ok(pid_str) => {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
                    println!("Stopped (PID {})", pid);
                    let _ = fs::remove_file(pid_file);
                } else {
                    eprintln!("Process {} not running", pid);
                    let _ = fs::remove_file(pid_file);
                }
            } else {
                eprintln!("Invalid PID file");
            }
        }
        Err(_) => {
            eprintln!("Not running (no PID file)");
        }
    }
    exit(0);
}

#[cfg(not(unix))]
fn embedded_stop(pid_file: &str) {
    eprintln!("Stop command not supported on this platform");
    eprintln!("PID file: {}", pid_file);
    exit(1);
}

/// Check status of a daemonized instance.
fn embedded_status(pid_file: &str) {
    match fs::read_to_string(pid_file) {
        Ok(pid_str) => {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                #[cfg(unix)]
                {
                    if unsafe { libc::kill(pid, 0) } == 0 {
                        println!("Running (PID {})", pid);
                    } else {
                        println!("Not running (stale PID file, was PID {})", pid);
                    }
                }
                #[cfg(not(unix))]
                {
                    println!("PID file exists (PID {})", pid);
                }
            } else {
                println!("Invalid PID file");
            }
        }
        Err(_) => {
            println!("Not running (no PID file)");
        }
    }
    exit(0);
}

