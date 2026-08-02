//! RustCFML configuration (`.cfconfig.json`).
//!
//! Engine-agnostic schema sharing the Ortus CFConfig filename convention with
//! the BoxLang-style flat, declarative layout. Unknown keys are silently
//! ignored so the same file can be consumed by Lucee, BoxLang, and RustCFML.
//!
//! Load entry points:
//!   * [`RustCfmlConfig::load`] — resolve the first matching file and parse it
//!   * [`RustCfmlConfig::from_str`] — parse JSON directly (used by tests and the
//!     embedded-VFS path for `--build` binaries)
//!
//! After parse, [`RustCfmlConfig::expand_env`] walks every string field and
//! resolves Lucee-compatible `${VAR:default}` placeholders against the process
//! environment.

pub mod env;
pub mod resolve;
pub mod schema;

pub use env::expand_env_vars;
pub use resolve::{resolve_config_path, LoadMode};
pub use schema::{
    CacheCfg, CacheProperties, DatasourceCfg, DebuggingCfg, Discovery, LoggerCfg, LoggingCfg,
    MailServerCfg, ObservabilityCfg, OtelCfg, OtelMetricsCfg, ProfilerCfg, RuntimeCfg,
    RustCfmlConfig, SecurityCfg, ServerCfg, UrlRewritingCfg,
};

/// Re-export of `serde_json::Value` used when callers want to convert the
/// config into another runtime's value type (e.g. CfmlValue) via a generic
/// JSON intermediate. Cheap because the underlying serializer is the same
/// one used during parse.
pub use serde_json::{to_value as to_json_value, Value as JsonValue};

use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl RustCfmlConfig {
    /// Search the supplied paths in order and parse the first file that exists.
    /// Returns the default config if no file is found.
    pub fn load(search_paths: &[std::path::PathBuf]) -> Result<Self, ConfigError> {
        let Some(found) = resolve::resolve_config_path(search_paths) else {
            log::debug!("no .cfconfig.json found in {} search paths", search_paths.len());
            return Ok(Self::default());
        };
        log::info!("loading cfconfig from {}", found.display());
        Self::from_file(&found)
    }

    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let bytes = std::fs::read(path).map_err(|source| ConfigError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let mut cfg: Self = serde_json::from_slice(&bytes).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        cfg.expand_env();
        cfg.source_path = Some(path.to_path_buf());
        Ok(cfg)
    }

    pub fn from_str(json: &str) -> Result<Self, ConfigError> {
        let mut cfg: Self =
            serde_json::from_str(json).map_err(|source| ConfigError::Parse {
                path: "<string>".to_string(),
                source,
            })?;
        cfg.expand_env();
        Ok(cfg)
    }

    /// Overlay an application-level config (raw JSON, as found in a
    /// `.cfconfig.json` beside an `Application.cfc`) on top of this server
    /// baseline, returning the merged result. Only keys actually present in
    /// `app_json` override the baseline; absent keys inherit. Object sections
    /// (`datasources`, `mappings`, `caches`) merge key-by-key so the app adds to
    /// or overrides individual baseline entries; scalars and arrays replace.
    ///
    /// The app's `server` section is **ignored**: server-level config (port,
    /// body size, welcome files) is owned by the baseline, never per-application.
    ///
    /// Takes a pre-parsed `serde_json::Value` rather than a path so the caller
    /// can source the bytes through its VFS (real FS or a bundled archive).
    pub fn overlay_app_json(
        &self,
        mut app_json: serde_json::Value,
    ) -> Result<Self, ConfigError> {
        if let Some(obj) = app_json.as_object_mut() {
            obj.remove("server");
            // The baseline serialises every datasource with its canonical
            // `driver` key. If the app overlay declares the same datasource the
            // Lucee way (`dbdriver`/`type`), the post-merge object would carry
            // BOTH `driver` (from the baseline) and the alias — which serde
            // rejects as a duplicate field. Fold the alias onto `driver` here so
            // the overlay value wins and only one key survives the merge.
            // The baseline serialises mappings under the canonical `mappings`
            // key. If the app overlay uses the `CFMappings` alias, the merged
            // object would carry BOTH keys → serde alias duplicate. Fold the
            // alias into `mappings` (alias values win on per-key conflict).
            if let Some(cf) = obj.remove("CFMappings") {
                let slot = obj
                    .entry("mappings".to_string())
                    .or_insert(serde_json::Value::Object(Default::default()));
                deep_merge(slot, &cf);
            }
            if let Some(datasources) = obj.get_mut("datasources").and_then(|d| d.as_object_mut()) {
                for (_, ds) in datasources.iter_mut() {
                    if let Some(ds_obj) = ds.as_object_mut() {
                        for alias in ["dbdriver", "type"] {
                            if let Some(v) = ds_obj.remove(alias) {
                                ds_obj.entry("driver").or_insert(v);
                            }
                        }
                    }
                }
            }
        }
        let mut base_json = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        deep_merge(&mut base_json, &app_json);
        let mut cfg: Self =
            serde_json::from_value(base_json).map_err(|source| ConfigError::Parse {
                path: "<app-overlay>".to_string(),
                source,
            })?;
        cfg.expand_env();
        Ok(cfg)
    }
}

/// Recursively merge `overlay` into `base`. Object values merge key-by-key;
/// any non-object value (scalar or array) in `overlay` replaces `base`.
fn deep_merge(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                deep_merge(base_map.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (base_slot, overlay_val) => {
            *base_slot = overlay_val.clone();
        }
    }
}
