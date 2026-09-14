//! MCP transports (native-only).
//!
//! `cfml-vm/src/mcp/` owns the protocol; this module owns the wires. Three
//! transports share one engine ([`engine::handle`]), so a spec rule is
//! implemented once:
//!
//! - [`http`] — Streamable HTTP (`POST`/`GET`/`DELETE /mcp/{name}`), the
//!   transport every modern client prefers.
//! - [`stdio`] — `rustcfml mcp <name>`, launched as a subprocess by a client.
//!
//! Discovery mirrors WebSockets: `<docroot>/mcp/<name>.cfc` is the server
//! reachable at `/mcp/<name>`, and the component's attributes are read with a
//! cheap source scan so an unknown path is rejected without spinning a VM.

pub(crate) mod dispatch;
pub(crate) mod engine;
pub(crate) mod http;
pub(crate) mod sse;
pub(crate) mod stdio;

use std::path::Path;
use std::sync::Arc;

use cfml_common::vfs::Vfs;

/// One resolved MCP server CFC.
#[derive(Clone, Debug)]
pub(crate) struct ServerInfo {
    /// URL/CLI name (`docs` for `/mcp/docs`).
    pub(crate) name: String,
    /// Server id used as the registry fan-out key, and reported to clients.
    pub(crate) id: String,
    pub(crate) cfc_path: String,
}

/// Resolve `<docroot>/mcp/<name>.cfc`.
///
/// Rejects anything that could escape the directory. Returns `None` when there
/// is no such CFC, which the HTTP layer turns back into ordinary page handling
/// — a registered `/mcp/{name}` route must not shadow a real page at that path.
pub(crate) fn resolve_server(
    doc_root: &Path,
    vfs: &Arc<dyn Vfs>,
    name: &str,
) -> Option<ServerInfo> {
    let name = name.trim_start_matches('/');
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return None;
    }
    let path = doc_root.join("mcp").join(format!("{name}.cfc"));
    let cfc_path = path.to_string_lossy().to_string();
    if !vfs.exists(&cfc_path) {
        return None;
    }
    Some(ServerInfo {
        id: format!("/mcp/{}", name.to_lowercase()),
        name: name.to_string(),
        cfc_path,
    })
}

/// True when this app has an `mcp/` directory at all. The `/mcp/{name}` routes
/// are only registered when it does, so an app that does not use MCP keeps the
/// path free for its own pages.
pub(crate) fn app_has_mcp(doc_root: &Path, vfs: &Arc<dyn Vfs>) -> bool {
    let dir = doc_root.join("mcp");
    vfs.exists(&dir.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::vfs::RealFs;
    use std::path::PathBuf;

    /// Self-cleaning temp directory — `tempfile` is not a dependency here and
    /// path-traversal safety is worth a test on its own.
    struct TempDir(PathBuf);

    impl TempDir {
        fn with_server() -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustcfml-mcp-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("mcp")).expect("mkdir");
            std::fs::write(path.join("mcp").join("docs.cfc"), "component {}").expect("write");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn vfs() -> Arc<dyn Vfs> {
        Arc::new(RealFs)
    }

    #[test]
    fn resolves_a_server_cfc_by_name() {
        let dir = TempDir::with_server();
        let info = resolve_server(&dir.0, &vfs(), "docs").expect("resolved");
        assert_eq!(info.id, "/mcp/docs");
        assert!(info.cfc_path.ends_with("docs.cfc"));
        assert!(app_has_mcp(&dir.0, &vfs()));
    }

    #[test]
    fn unknown_names_and_traversal_are_refused() {
        let dir = TempDir::with_server();
        let vfs = vfs();
        assert!(resolve_server(&dir.0, &vfs, "nope").is_none());
        assert!(resolve_server(&dir.0, &vfs, "..").is_none());
        assert!(resolve_server(&dir.0, &vfs, "../secrets").is_none());
        assert!(resolve_server(&dir.0, &vfs, "a/b").is_none());
        assert!(resolve_server(&dir.0, &vfs, "").is_none());
    }

    #[test]
    fn an_app_without_an_mcp_directory_registers_no_routes() {
        let empty = std::env::temp_dir().join("rustcfml-mcp-absent");
        assert!(!app_has_mcp(&empty, &vfs()));
    }
}
