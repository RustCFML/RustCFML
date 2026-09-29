//! `getPageContext().getConfig()` — the slice of Lucee's `ConfigWeb` that
//! monitoring code reads for connection-pool gauges:
//!
//! ```cfml
//! var cfg  = getPageContext().getConfig();
//! var pool = cfg.getDatasourceConnectionPool( cfg.getDatasource( "preside" ), nullValue(), nullValue() );
//! pool.getNumActive(); pool.getNumIdle(); pool.getMaxTotal();
//! ```
//!
//! `getMaxTotal()` is real: each driver's pool has a fixed maximum. The
//! connection COUNTS are not observable — the MySQL pool keeps them private —
//! so `getNumActive()` / `getNumIdle()` return an EMPTY STRING. That is a
//! deliberate stand-in (docs/known-issues.md §116): code that formats the value
//! with `IsNumeric()` (preside-ext-k8s-essentials' Prometheus collector) emits
//! `NaN`, which Prometheus records as "no data" — never a 0 that would read as
//! "no connections in use". An empty string, not null: passing null to an
//! optional argument makes it undefined, and the formatter would then throw.

use super::*;
use crate::shim_util::{shim, field};

pub(crate) const CONFIG_CLASS: &str = "lucee.runtime.config.configwebimpl";
const DATASOURCE_CLASS: &str = "lucee.runtime.db.datasourceimpl";
const POOL_CLASS: &str = "lucee.runtime.config.datasourceconnpool";

pub(crate) fn handles(class_lower: &str) -> bool {
    matches!(class_lower, CONFIG_CLASS | DATASOURCE_CLASS | POOL_CLASS)
}

/// The fixed maximum of the pool a connection URL is served by (see the pool
/// builders in cfml-stdlib). `None` when the driver is not one we pool.
fn pool_max_for_url(url: &str) -> Option<i64> {
    let u = url.to_ascii_lowercase();
    if u.starts_with("mysql:") || u.starts_with("mariadb:") {
        Some(100)
    } else if u.starts_with("postgres") || u.starts_with("sqlite") || u.starts_with("mssql") || u.starts_with("sqlserver") {
        Some(10)
    } else if u.ends_with(".db") || u.ends_with(".sqlite") || u == ":memory:" {
        Some(10)
    } else {
        None
    }
}

impl CfmlVirtualMachine {
    pub(crate) fn page_config_shim(&self) -> CfmlValue {
        CfmlValue::strukt(shim("lucee.runtime.config.ConfigWebImpl"))
    }

    /// The datasource named `name`: one configured in `.cfconfig.json`, or
    /// registered for the application (`this.datasources`, `cfadmin
    /// updateDatasource`). Lucee's `ConfigWeb.getDatasource` sees only the
    /// configured ones; accepting the application's too is a superset, since
    /// here the two cannot be told apart once registered.
    fn lookup_datasource_url(&self, name: &str) -> Option<(String, Option<i32>)> {
        let lower = name.to_lowercase();
        let cfg = self
            .app_cfconfig
            .as_ref()
            .map(|c| c.as_ref())
            .or_else(|| self.server_state.as_ref().map(|s| s.cfconfig.as_ref()));
        if let Some(cfg) = cfg {
            if let Some((_, ds)) = cfg.datasources.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
                let limit = (ds.connection_limit > 0).then_some(ds.connection_limit);
                return Some((ds.connection_url().unwrap_or_default(), limit));
            }
        }
        self.app_datasources.get(&lower).map(|u| (u.clone(), None))
    }

    pub(crate) fn dispatch_lucee_config(
        &mut self,
        class_lower: &str,
        method: &str,
        args: Vec<CfmlValue>,
        object: &CfmlValue,
    ) -> CfmlResult {
        match (class_lower, method) {
            (CONFIG_CLASS, "getdatasource") => {
                let name = args.first().map(|v| v.as_string()).unwrap_or_default();
                let Some((url, limit)) = self.lookup_datasource_url(&name) else {
                    return Err(CfmlError::database(format!("datasource [{}] doesn't exist", name)));
                };
                let mut m = shim("lucee.runtime.db.DataSourceImpl");
                m.insert("__name".to_string(), CfmlValue::string(name));
                m.insert("__url".to_string(), CfmlValue::string(url));
                m.insert("__limit".to_string(), CfmlValue::Int(limit.map(i64::from).unwrap_or(-1)));
                Ok(CfmlValue::strukt(m))
            }
            // getDatasourceConnectionPool(dataSource, user, password)
            (CONFIG_CLASS, "getdatasourceconnectionpool") => {
                let ds = args.first().cloned().unwrap_or(CfmlValue::Null);
                if field(&ds, "__url").is_none() {
                    return Err(CfmlError::runtime(
                        "getDatasourceConnectionPool() needs a datasource from getDatasource()".to_string(),
                    ));
                }
                let mut m = shim("lucee.runtime.config.DatasourceConnPool");
                m.insert("__url".to_string(), field(&ds, "__url").unwrap_or(CfmlValue::Null));
                Ok(CfmlValue::strukt(m))
            }
            (DATASOURCE_CLASS, "getname") => Ok(field(object, "__name").unwrap_or(CfmlValue::Null)),
            (DATASOURCE_CLASS, "getconnectionlimit") => Ok(field(object, "__limit").unwrap_or(CfmlValue::Int(-1))),
            (POOL_CLASS, "getmaxtotal") => {
                let url = field(object, "__url").map(|v| v.as_string()).unwrap_or_default();
                Ok(pool_max_for_url(&url).map(CfmlValue::Int).unwrap_or_else(|| CfmlValue::string(String::new())))
            }
            // Not observable (see the module doc): unknown, not zero.
            (POOL_CLASS, "getnumactive") | (POOL_CLASS, "getnumidle") => Ok(CfmlValue::string(String::new())),
            (class, other) => Err(CfmlError::runtime(format!(
                "{}.{}() is not supported by RustCFML",
                class, other
            ))),
        }
    }
}
