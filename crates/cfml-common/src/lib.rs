//! Common utilities for RustCFML

/// RustCFML workspace version (cfml-common inherits `version.workspace = true`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod cfhttp;
pub mod charset;
pub mod clock;
pub mod component;
pub mod cycle_gc;
pub mod dynamic;
pub mod encodings;
pub mod introspection;
pub mod locale;
pub mod logging;
pub mod position;
pub mod session_cookie;
pub mod vfs;
pub mod vm;
