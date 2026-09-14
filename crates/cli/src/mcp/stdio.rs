//! stdio transport — `rustcfml mcp <name> [webroot]`.
//!
//! The client launches the engine as a subprocess and speaks newline-delimited
//! JSON-RPC over the pipes. The spec's hard rule is that **nothing** which is
//! not an MCP message may reach stdout, and an interpreter has a great many
//! ways to print: `writeOutput` from a tool, a `println!` deep in the engine, a
//! panic message, a native shim writing to fd 1 directly. Discipline cannot
//! cover all of that, so on unix the real stdout is duplicated to a private
//! descriptor used only by the writer and fd 1 is pointed at stderr. Every
//! stray write then lands harmlessly in the client's log instead of corrupting
//! the protocol stream.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cfml_common::vfs::{RealFs, Vfs};
use cfml_vm::mcp::protocol::{self, Outgoing, RpcId};
use cfml_vm::ServerState;

use super::dispatch::McpRuntime;
use super::engine::{self, Ctx, Handled};

/// Entry point for the `mcp` subcommand, dispatched before clap sees a
/// positional filename (the same shape as `rustcfml ext …`).
pub(crate) fn main(args: &[String]) -> i32 {
    let Some(out) = claim_stdout() else { return 1 };
    main_with(args, None, out)
}

/// The same subcommand inside a `--build` binary.
///
/// A bundled app is exactly what a client launches from its config
/// (`"command": "/path/to/myapp", "args": ["mcp", "docs"]`), so the
/// subcommand has to exist there too. The difference is the filesystem: the
/// server CFC lives in the embedded archive, so the VFS and web root are
/// injected rather than resolved from disk.
pub(crate) fn embedded(
    args: &[String],
    vfs: Arc<dyn Vfs>,
    base_dir: &str,
    out: StdoutGuard,
) -> i32 {
    main_with(args, Some((vfs, PathBuf::from(base_dir))), out)
}

/// Take exclusive ownership of stdout before anything can write to it.
/// Separate from the rest so an embedded binary can claim it on its very
/// first line, ahead of extension loading and banner printing.
pub(crate) fn claim_stdout() -> Option<StdoutGuard> {
    match StdoutGuard::claim() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("rustcfml mcp: could not secure stdout: {e}");
            None
        }
    }
}

fn main_with(
    args: &[String],
    embedded: Option<(Arc<dyn Vfs>, PathBuf)>,
    mut out: StdoutGuard,
) -> i32 {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "Usage: rustcfml mcp <name> [webroot]\n\n\
             Serves <webroot>/mcp/<name>.cfc over the MCP stdio transport.\n\
             The webroot defaults to the current directory."
        );
        return if args.is_empty() { 2 } else { 0 };
    }
    let name = args[0].clone();
    let embedded_vfs = embedded.as_ref().map(|(vfs, _)| vfs.clone());
    let (vfs, webroot): (Arc<dyn Vfs>, PathBuf) = match embedded {
        // A bundled app serves its own embedded files; a `webroot` argument
        // would be meaningless there.
        Some((vfs, base)) => (vfs, base),
        None => (
            Arc::new(RealFs),
            args.get(1).map(PathBuf::from).unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            }),
        ),
    };
    let Some(info) = super::resolve_server(&webroot, &vfs, &name) else {
        eprintln!(
            "rustcfml mcp: no MCP server named [{name}] — expected {}",
            webroot.join("mcp").join(format!("{name}.cfc")).display()
        );
        return 1;
    };

    let cfconfig = Arc::new(load_cfconfig(&webroot, embedded_vfs.as_ref()));
    let mut server_state = ServerState::with_config(false, cfconfig.clone());
    server_state.webroot = Some(webroot.clone());
    // On stdio the caller IS whoever launched this process, so they are
    // authenticated; their roles come from configuration.
    let caller = super::auth::Caller::stdio(&cfconfig.mcp);
    let runtime = McpRuntime { server_state, vfs, sandbox: false };

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("rustcfml mcp: could not start the runtime: {e}");
            return 1;
        }
    };

    let stdin = BufReader::new(std::io::stdin());
    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("rustcfml mcp: stdin closed: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        for reply in rt.block_on(handle_line(&runtime, &info, line.as_bytes(), &caller)) {
            if let Err(e) = out.write_message(&reply) {
                eprintln!("rustcfml mcp: could not write to stdout: {e}");
                return 1;
            }
        }
    }
    0
}

/// Decode one line and produce the messages to write back.
///
/// Messages are handled in order and one at a time. stdio is a single client
/// on a single pipe, so serial handling keeps ordering obvious; concurrency
/// here would only matter for a client that pipelines long tool calls.
async fn handle_line(
    runtime: &McpRuntime,
    info: &super::ServerInfo,
    body: &[u8],
    caller: &super::auth::Caller,
) -> Vec<Outgoing> {
    let messages = match protocol::decode(body) {
        Ok(m) => m,
        Err((code, message)) => {
            return vec![Outgoing::error(RpcId::Null, code, message)]
        }
    };
    // One implicit session: the process *is* the conversation, so there is no
    // session id on the wire and no way for a second client to reach it.
    let session = "stdio".to_string();
    let mut capabilities = cfml_vm::mcp::ClientCapabilities::default();
    let mut out = Vec::new();

    for msg in messages {
        if let cfml_vm::mcp::Incoming::Request { method, params, .. } = &msg {
            if method == protocol::methods::INITIALIZE {
                capabilities = cfml_vm::mcp::ClientCapabilities::from_json(
                    params.get("capabilities").unwrap_or(&serde_json::Value::Null),
                );
            }
        }
        let ctx = Ctx {
            runtime: runtime.clone(),
            info: info.clone(),
            session: Some(session.clone()),
            caller: caller.clone(),
            capabilities: capabilities.clone(),
            stream: None,
            manifest: None,
        };
        match engine::handle(&ctx, msg).await {
            Handled::Accepted => {}
            Handled::Reply(reply) => out.push(*reply),
            // stdio has no session header to carry, so an initialize is just
            // its reply.
            Handled::Initialized { reply, .. } => out.push(*reply),
        }
    }
    out
}

/// Load `.cfconfig.json` the way serve mode does, so `mcp.stdioRoles`, tool
/// filters and `mcpServers` all behave identically on this transport. An
/// embedded binary reads it out of its own archive.
fn load_cfconfig(webroot: &Path, embedded: Option<&Arc<dyn Vfs>>) -> cfml_config::RustCfmlConfig {
    if let Some(vfs) = embedded {
        return crate::load_embedded_cfconfig(vfs.as_ref(), &webroot.to_string_lossy());
    }
    let mut search = vec![webroot.to_path_buf()];
    if let Ok(cwd) = std::env::current_dir() {
        if cwd != webroot {
            search.push(cwd);
        }
    }
    if let Some(dir) = cfml_config::resolve::exe_dir() {
        search.push(dir);
    }
    cfml_config::RustCfmlConfig::load(&search).unwrap_or_else(|e| {
        // stderr, never stdout: a warning on fd 1 would corrupt the stream.
        eprintln!("rustcfml mcp: ignoring unreadable .cfconfig.json: {e}");
        Default::default()
    })
}

/// Exclusive ownership of the real stdout.
pub(crate) struct StdoutGuard {
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    out: std::io::Stdout,
}

impl StdoutGuard {
    #[cfg(unix)]
    fn claim() -> std::io::Result<Self> {
        use std::os::fd::FromRawFd;
        // SAFETY: fd 1 is open at process start; `dup` gives us a private copy
        // and `dup2` then redirects the original so nothing else can reach the
        // client's message stream.
        let private = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if private < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { file: unsafe { std::fs::File::from_raw_fd(private) } })
    }

    /// On non-unix there is no fd-level guard, so engine output could in
    /// principle interleave. Windows stdio support is tracked separately; the
    /// transport still works for a well-behaved server.
    #[cfg(not(unix))]
    fn claim() -> std::io::Result<Self> {
        Ok(Self { out: std::io::stdout() })
    }

    fn write_message(&mut self, msg: &Outgoing) -> std::io::Result<()> {
        let line = msg.to_line();
        debug_assert!(!line.contains('\n'), "stdio framing forbids embedded newlines");
        #[cfg(unix)]
        {
            self.file.write_all(line.as_bytes())?;
            self.file.write_all(b"\n")?;
            self.file.flush()
        }
        #[cfg(not(unix))]
        {
            let mut lock = self.out.lock();
            lock.write_all(line.as_bytes())?;
            lock.write_all(b"\n")?;
            lock.flush()
        }
    }
}
