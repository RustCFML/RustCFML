//! `cfadmin` — the Lucee Administrator API tag, shimmed onto the engine state
//! RustCFML actually has.
//!
//! RustCFML has no Administrator, and this is not one. But most of what real
//! code asks `cfadmin` for is not administration — it is the engine reporting
//! its *own* configuration, and we hold that configuration already:
//!
//! | action | reads/writes |
//! |---|---|
//! | `getDebug` / `updateDebug` | `vm.debug_config` — the Lucee-modelled `debugging` block |
//! | `updateDebugSetting` | `debug_config.max_records` (Lucee's `maxLogs`) |
//! | `getDebugEntry` | the debug-template registry (we have exactly one template, not a registry) |
//! | `getCompilerSettings` / `updateCompilerSettings` | the `runtime` cfconfig block |
//! | `updateDatasource` | `vm.app_datasources` — the same registry `this.datasources` seeds |
//!
//! Before this existed the tag form threw "not implemented" and the SCRIPT form
//! (`admin action="…" …;`) silently did nothing at all — see
//! `UNSUPPORTED_TAG_STATEMENTS` in the parser. That silence was not merely a
//! missing feature: Preside's `_setupInjectedDatasource()` registers the
//! application's datasource through `admin action="updateDatasource"` whenever
//! the datasource arrives from injected env config (every containerised deploy),
//! so the datasource was never registered and every later query failed with
//! "datasource not found" and nothing explaining why.
//!
//! # Two rules for anything added here
//!
//! 1. **Report the real state, never a convenient one.** `getDebug` returning a
//!    hardcoded `debug=true` would make Preside's performance-analyser extension
//!    walk on into Lucee's debugger internals (`getPageContext().getDebugger()`)
//!    looking for a request log we do not keep. Returning our actual
//!    `debugging.enabled` lands it on its own "debugging disabled" panel, which
//!    is both true and a page that renders.
//! 2. **An action we cannot map THROWS.** Returning an empty struct for an
//!    unknown action is how this whole class of bug started. A caller that wants
//!    feature detection already wraps the call in `try`/`catch` — that is exactly
//!    what `LuceeAdminApiWrapper.canConnect()` does, and a throw is what makes it
//!    correctly report "no admin here".

use super::*;

/// Names this module handles.
#[inline]
pub(crate) fn handles(name_lower: &str) -> bool {
    matches!(name_lower, "cfadmin" | "__cfadmin")
}

/// Case-insensitive fetch of a string attribute.
fn attr(opts: &cfml_common::dynamic::CfmlStruct, key: &str) -> Option<CfmlValue> {
    opts.get_ci(key)
}

fn attr_str(opts: &cfml_common::dynamic::CfmlStruct, key: &str) -> String {
    attr(opts, key).map(|v| v.as_string()).unwrap_or_default()
}

fn attr_bool(opts: &cfml_common::dynamic::CfmlStruct, key: &str) -> Option<bool> {
    attr(opts, key).map(|v| v.is_true())
}

impl CfmlVirtualMachine {
    pub(crate) fn dispatch_admin(&mut self, _name_lower: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let Some(CfmlValue::Struct(opts)) = args.first() else {
            return Err(CfmlError::runtime(
                "cfadmin requires an action".to_string(),
            ));
        };
        let action = attr_str(opts, "action");
        if action.trim().is_empty() {
            return Err(CfmlError::runtime(
                "cfadmin requires an action".to_string(),
            ));
        }
        match action.to_lowercase().as_str() {
            "getdebug" => self.admin_get_debug(),
            "updatedebug" => self.admin_update_debug(opts),
            "updatedebugsetting" => self.admin_update_debug_setting(opts),
            // Lucee's registry of debug-output TEMPLATES. We have a fixed set
            // (`debugging.template`), not a registry of user CFCs, so there is
            // never a user-registered entry to report. An empty list is the
            // truthful answer and is what every caller's `?:` fallbacks expect;
            // it is NOT a stand-in for "we didn't look".
            "getdebugentry" => Ok(CfmlValue::array(Vec::new())),
            "getcompilersettings" => Ok(self.admin_get_compiler_settings()),
            "updatecompilersettings" => self.admin_update_compiler_settings(opts),
            "updatedatasource" => self.admin_update_datasource(opts),
            other => Err(CfmlError::runtime(format!(
                "cfadmin action [{}] is not supported by RustCFML. RustCFML has no \
                 Administrator; the supported actions report or update the engine's own \
                 configuration: getDebug, updateDebug, updateDebugSetting, getDebugEntry, \
                 getCompilerSettings, updateCompilerSettings, updateDatasource.",
                other
            ))),
        }
    }

    /// The `getCompilerSettings` / `updateCompilerSettings` / `updateDatasource`
    /// half, which every build has. The debug actions live in the
    /// feature-gated `impl` blocks below.

    /// The `runtime` cfconfig block in Lucee's `getCompilerSettings` clothing.
    fn admin_get_compiler_settings(&self) -> CfmlValue {
        let runtime = self
            .app_cfconfig
            .as_ref()
            .map(|c| c.runtime.clone())
            .or_else(|| self.server_state.as_ref().map(|ss| ss.cfconfig.runtime.clone()))
            .unwrap_or_default();
        let mut out = ValueMap::default();
        out.insert(
            "dotNotationUpperCase".to_string(),
            CfmlValue::Bool(runtime.dot_notation_upper_case),
        );
        out.insert(
            "nullSupport".to_string(),
            CfmlValue::Bool(runtime.null_support),
        );
        out.insert(
            "suppressWSBeforeArg".to_string(),
            CfmlValue::Bool(runtime.whitespace_compression_enabled),
        );
        // RustCFML reads every template as UTF-8; there is no per-charset
        // template decoder to report.
        out.insert(
            "templateCharset".to_string(),
            CfmlValue::string("UTF-8".to_string()),
        );
        CfmlValue::strukt(out)
    }

    /// `updateCompilerSettings` — accepts a request that matches what the engine
    /// already does and REFUSES one that does not, rather than reporting success
    /// for a setting it cannot change.
    ///
    /// Preside's `Bootstrap._ensureCaseSensitiveStructSettingsAreActive()` is the
    /// caller that matters: it asks for `dotNotationUpperCase=false` so struct
    /// keys keep their case. RustCFML preserves key case unconditionally, so the
    /// request is already satisfied — and in practice Preside never gets here,
    /// because the block is gated on the engine uppercasing keys in the first
    /// place.
    fn admin_update_compiler_settings(
        &mut self,
        opts: &cfml_common::dynamic::CfmlStruct,
    ) -> CfmlResult {
        if let Some(true) = attr_bool(opts, "dotNotationUpperCase") {
            return Err(CfmlError::runtime(
                "cfadmin action [updateCompilerSettings] cannot set dotNotationUpperCase=true: \
                 RustCFML preserves struct key case and has no uppercasing mode."
                    .to_string(),
            ));
        }
        // Everything else in this action is a compile-time engine setting that
        // cannot change inside a running request on either engine. Accepting the
        // call is honest only because the values we would set are the values we
        // already report; a request to CHANGE one is refused above.
        Ok(CfmlValue::Null)
    }

    /// `updateDatasource` — registers (or replaces) a datasource for this
    /// application, the same registry `this.datasources` and the cfconfig
    /// `datasources` block feed.
    ///
    /// Lucee's attribute names differ from ours in two places: the credentials
    /// arrive as `dbusername`/`dbpassword`, and the driver as `dbdriver`/`type`.
    /// `datasource_value_to_url` already understands `type`/`dbdriver`, so only
    /// the credentials need aliasing.
    fn admin_update_datasource(&mut self, opts: &cfml_common::dynamic::CfmlStruct) -> CfmlResult {
        let name = attr_str(opts, "name");
        if name.trim().is_empty() {
            return Err(CfmlError::runtime(
                "cfadmin action [updateDatasource] requires a name".to_string(),
            ));
        }
        // `type` on the TAG is the admin context ("web"/"server"); on a
        // datasource it is the driver. `dbdriver` is unambiguous, so when it is
        // present `type` is dropped rather than mistaken for the driver.
        let has_dbdriver = opts
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("dbdriver"));
        let mut def = ValueMap::default();
        for (k, v) in opts.iter() {
            let lk = k.to_lowercase();
            // The tag's own attributes, not the datasource's. `password` here is
            // the ADMIN password; the datasource's arrives as `dbpassword`.
            if matches!(
                lk.as_str(),
                "action" | "returnvariable" | "password" | "remoteclients"
            ) {
                continue;
            }
            if lk == "type" && has_dbdriver {
                continue;
            }
            let mapped = match lk.as_str() {
                "dbusername" => "username",
                "dbpassword" => "password",
                _ => k.as_str(),
            };
            def.insert(mapped.to_string(), v.clone());
        }
        let def = CfmlValue::strukt(def);
        match Self::datasource_value_to_url(&def) {
            Some((url, timeout)) => {
                self.register_ds_timeout_via_builtin(&url, timeout);
                self.app_datasources.insert(name.to_lowercase(), url);
                Ok(CfmlValue::Null)
            }
            None => Err(CfmlError::runtime(format!(
                "cfadmin action [updateDatasource] could not build a connection for [{}]: \
                 a driver (dbdriver/type) and either a database/host pair or a \
                 connectionString are required.",
                name
            ))),
        }
    }
}

/// The debug actions read and write `debug_config`, which exists only in builds
/// with the `observability` feature (not the wasm targets).
#[cfg(feature = "observability")]
impl CfmlVirtualMachine {
    /// Lucee's `getDebug` shape, filled from `debug_config`. The feature-flag
    /// key names are Lucee's own and happen to be exactly our `DebugFieldsCfg`
    /// field names, so this is a rename-free mapping.
    fn admin_get_debug(&self) -> CfmlResult {
        let cfg = &self.debug_config;
        let mut out = ValueMap::default();
        out.insert("debug".to_string(), CfmlValue::Bool(cfg.enabled));
        out.insert(
            "database".to_string(),
            CfmlValue::Bool(cfg.fields.database),
        );
        out.insert(
            "exception".to_string(),
            CfmlValue::Bool(cfg.fields.exception),
        );
        out.insert("tracing".to_string(), CfmlValue::Bool(cfg.fields.tracing));
        out.insert("timer".to_string(), CfmlValue::Bool(cfg.fields.timer));
        out.insert(
            "implicitAccess".to_string(),
            CfmlValue::Bool(cfg.fields.implicit_access),
        );
        out.insert(
            "queryUsage".to_string(),
            CfmlValue::Bool(cfg.fields.query_usage),
        );
        out.insert("dump".to_string(), CfmlValue::Bool(cfg.fields.dump));
        // Lucee names the SELECTED debug template here. Ours is the built-in
        // renderer id (`modern`/`classic`/`simple`/`comment`/`none`).
        out.insert(
            "debugTemplate".to_string(),
            CfmlValue::string(cfg.template.clone()),
        );
        out.insert(
            "maxLogs".to_string(),
            CfmlValue::Int(cfg.max_records as i64),
        );
        Ok(CfmlValue::strukt(out))
    }

    /// `updateDebug` — applies to the LIVE config for this request. It does not
    /// rewrite `.cfconfig.json`: a tag that silently edited a file on disk would
    /// be a larger surprise than one whose effect ends with the request, and
    /// Lucee's own per-web-context update is not the server config either.
    fn admin_update_debug(&mut self, opts: &cfml_common::dynamic::CfmlStruct) -> CfmlResult {
        if let Some(v) = attr_bool(opts, "debug") {
            self.debug_config.enabled = v;
        }
        if let Some(v) = attr_bool(opts, "database") {
            self.debug_config.fields.database = v;
        }
        if let Some(v) = attr_bool(opts, "exception") {
            self.debug_config.fields.exception = v;
        }
        if let Some(v) = attr_bool(opts, "tracing") {
            self.debug_config.fields.tracing = v;
        }
        if let Some(v) = attr_bool(opts, "timer") {
            self.debug_config.fields.timer = v;
        }
        if let Some(v) = attr_bool(opts, "implicitAccess") {
            self.debug_config.fields.implicit_access = v;
        }
        if let Some(v) = attr_bool(opts, "queryUsage") {
            self.debug_config.fields.query_usage = v;
        }
        if let Some(v) = attr_bool(opts, "dump") {
            self.debug_config.fields.dump = v;
        }
        Ok(CfmlValue::Null)
    }

    fn admin_update_debug_setting(&mut self, opts: &cfml_common::dynamic::CfmlStruct) -> CfmlResult {
        // Lucee's `maxLogs` caps retained debug records; ours is
        // `debugging.maxRecords`. `0` is Lucee's "no cap"; we keep the
        // configured value rather than inventing an unbounded buffer.
        if let Some(max) = attr(opts, "maxLogs") {
            let n = max.as_string().trim().parse::<usize>().unwrap_or(0);
            if n > 0 {
                self.debug_config.max_records = n;
            }
        }
        Ok(CfmlValue::Null)
    }
}

/// A build without the debugging subsystem has no debug config to report or
/// change, so these throw (rule 2 above) rather than invent one.
#[cfg(not(feature = "observability"))]
impl CfmlVirtualMachine {
    fn admin_get_debug(&self) -> CfmlResult {
        Err(Self::admin_no_debugging("getDebug"))
    }

    fn admin_update_debug(&mut self, _opts: &cfml_common::dynamic::CfmlStruct) -> CfmlResult {
        Err(Self::admin_no_debugging("updateDebug"))
    }

    fn admin_update_debug_setting(&mut self, _opts: &cfml_common::dynamic::CfmlStruct) -> CfmlResult {
        Err(Self::admin_no_debugging("updateDebugSetting"))
    }

    fn admin_no_debugging(action: &str) -> CfmlError {
        CfmlError::runtime(format!(
            "cfadmin action [{}] is not supported by this RustCFML build: it was \
             compiled without the debugging subsystem.",
            action
        ))
    }
}
