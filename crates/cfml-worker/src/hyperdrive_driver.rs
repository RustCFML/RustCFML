//! Cloudflare Hyperdrive driver for the cfml-stdlib dynamic-driver registry.
//!
//! End-to-end path:
//! 1. CFML calls `<cfquery datasource="main">...</cfquery>` or
//!    `queryExecute("...", [...], {datasource: "main"})`.
//! 2. `cfml-stdlib::fn_query_execute` looks up the dynamic registry by
//!    name, finds a `HyperdriveDriver` registered by
//!    `cfml_worker::handle_fetch`.
//! 3. We serialize `(name, sql, params)` to JSON and call the JSPI extern
//!    [`crate::jspi::hyperdrive_query_sync`].
//! 4. JSPI suspends wasm, the JS shim resolves the `env[name]` Hyperdrive
//!    binding, reads its `connectionString`, sniffs `postgres://` vs
//!    `mysql://`, awaits the query through `postgres` (postgres.js) or
//!    `mysql2/promise`, packs the response JSON back into wasm memory,
//!    resumes us.
//! 5. We parse the response and shape it into the CFML query value the rest
//!    of the language expects.
//!
//! Failure modes:
//! - Shim missing → user-friendly error mentioning the wiring step.
//! - SQL error → propagated as `CfmlError` with the original message.

#![cfg(target_arch = "wasm32")]

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use cfml_stdlib::DynamicDbDriver;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use worker::Hyperdrive;

pub struct HyperdriveDriver {
    name: String,
    /// Kept on the struct so the handler can hold the binding alive across
    /// the request. The JS shim itself looks up the binding by `name` on the
    /// `env` it captured at request start.
    #[allow(dead_code)]
    binding: Arc<Hyperdrive>,
}

impl HyperdriveDriver {
    pub fn new(name: impl Into<String>, binding: Arc<Hyperdrive>) -> Self {
        Self {
            name: name.into(),
            binding,
        }
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    datasource: &'a str,
    /// One or more statements to run in order on a single connection. For
    /// PostgreSQL the SQL has already been rewritten to `$1..$n` placeholders
    /// and split into single statements (see `cfml_stdlib::pg_sql`); for MySQL
    /// it's a single statement with native `?` placeholders.
    statements: Vec<WireStatement>,
}

#[derive(Serialize)]
struct WireStatement {
    sql: String,
    params: Vec<WireParam>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireParam {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl WireParam {
    fn from_cfml(v: &CfmlValue) -> Self {
        match v {
            CfmlValue::Null => WireParam::Null,
            CfmlValue::Bool(b) => WireParam::Bool(*b),
            CfmlValue::Int(i) => WireParam::Int(*i),
            CfmlValue::Double(d) => WireParam::Float(*d),
            CfmlValue::String(s) => WireParam::Str((**s).clone()),
            other => WireParam::Str(other.as_string()),
        }
    }
}

#[derive(Deserialize)]
struct WireResponse {
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    results: Option<Vec<serde_json::Map<String, serde_json::Value>>>,
    #[serde(default)]
    meta: Option<WireMeta>,
}

#[derive(Deserialize, Default)]
struct WireMeta {
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    rows_affected: i64,
    #[serde(default)]
    last_insert_id: i64,
}

impl DynamicDbDriver for HyperdriveDriver {
    fn execute(&self, sql: &str, params_arg: &CfmlValue, return_type: &str) -> CfmlResult {
        // The HyperdriveDriver serves both PostgreSQL and MySQL — the scheme is
        // only known from the binding's connection string. PostgreSQL needs
        // `?`→`$n` rewriting and multi-statement splitting (postgres.js
        // `unsafe()` accepts one parameterized command); MySQL keeps native `?`
        // placeholders untouched. See docs/compatibility-notes/postgres-*.md.
        let cs = self.binding.connection_string();
        let is_pg = cs.starts_with("postgres://") || cs.starts_with("postgresql://");

        let statements: Vec<WireStatement> = if is_pg {
            let split = !cfml_stdlib::pg_sql::is_pg_select(sql);
            let prepared = cfml_stdlib::pg_sql::prepare_pg_statements(sql, params_arg, split)?;
            prepared
                .into_iter()
                .map(|st| WireStatement {
                    sql: st.sql,
                    params: st.params.iter().map(WireParam::from_cfml).collect(),
                })
                .collect()
        } else {
            // CfmlArray::iter() yields owned CfmlValues (reference-typed array
            // snapshot), so borrow each for WireParam::from_cfml(&CfmlValue).
            let params = match params_arg {
                CfmlValue::Array(arr) => arr.iter().map(|v| WireParam::from_cfml(&v)).collect(),
                CfmlValue::Null => Vec::new(),
                single => vec![WireParam::from_cfml(single)],
            };
            vec![WireStatement {
                sql: sql.to_string(),
                params,
            }]
        };

        let req = WireRequest {
            datasource: &self.name,
            statements,
        };
        let request_json = serde_json::to_string(&req).map_err(|e| {
            CfmlError::runtime(format!(
                "cfquery (Hyperdrive '{}'): could not serialize request: {}",
                self.name, e
            ))
        })?;

        let response_json = crate::jspi::hyperdrive_query_sync(&request_json)?;
        let response: WireResponse = serde_json::from_str(&response_json).map_err(|e| {
            CfmlError::runtime(format!(
                "cfquery (Hyperdrive '{}'): malformed shim response: {}",
                self.name, e
            ))
        })?;

        if !response.success {
            return Err(CfmlError::runtime(format!(
                "cfquery (Hyperdrive '{}'): {}",
                self.name,
                response.error.unwrap_or_else(|| "unknown error".into())
            )));
        }

        let rows = response.results.unwrap_or_default();
        let meta = response.meta.unwrap_or_default();

        match return_type {
            "array" => Ok(rows_to_array(rows)),
            rt if rt.starts_with("struct:") => {
                let key = &rt["struct:".len()..];
                Ok(rows_to_struct_by_key(rows, key))
            }
            _ => Ok(rows_to_query(rows, &meta)),
        }
    }
}

fn json_to_cfml(v: serde_json::Value) -> CfmlValue {
    match v {
        serde_json::Value::Null => CfmlValue::Null,
        serde_json::Value::Bool(b) => CfmlValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CfmlValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                CfmlValue::Double(f)
            } else {
                CfmlValue::string(n.to_string())
            }
        }
        serde_json::Value::String(s) => CfmlValue::string(s),
        serde_json::Value::Array(arr) => {
            CfmlValue::array(arr.into_iter().map(json_to_cfml).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut m = ValueMap::default();
            for (k, val) in obj {
                m.insert(k, json_to_cfml(val));
            }
            CfmlValue::strukt(m)
        }
    }
}

fn rows_to_query(
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
    meta: &WireMeta,
) -> CfmlValue {
    let mut columns: Vec<String> = Vec::new();
    for row in &rows {
        for k in row.keys() {
            if !columns.iter().any(|c| c.eq_ignore_ascii_case(k)) {
                columns.push(k.clone());
            }
        }
    }

    let record_count = rows.len();

    let mut records: Vec<CfmlValue> = Vec::with_capacity(record_count);
    for row in rows {
        let mut rec = ValueMap::default();
        for col in &columns {
            let val = row
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(col))
                .map(|(_, v)| v.clone())
                .unwrap_or(serde_json::Value::Null);
            rec.insert(col.clone(), json_to_cfml(val));
        }
        records.push(CfmlValue::strukt(rec));
    }

    let mut q = ValueMap::default();
    q.insert("recordCount", CfmlValue::Int(record_count as i64));
    // columnList reports column names uppercased, matching Lucee/ACF.
    q.insert("columnList", CfmlValue::string(columns.iter().map(|c| c.to_uppercase()).collect::<Vec<_>>().join(",")));
    q.insert(
        "columns",
        CfmlValue::array(
            columns
                .iter()
                .map(|c| CfmlValue::string(c.clone()))
                .collect(),
        ),
    );
    q.insert("data", CfmlValue::array(records));

    let mut driver_meta = ValueMap::default();
    driver_meta.insert("duration_ms", CfmlValue::Double(meta.duration));
    driver_meta.insert(
        "rows_affected",
        CfmlValue::Int(meta.rows_affected),
    );
    driver_meta.insert(
        "last_insert_id",
        CfmlValue::Int(meta.last_insert_id),
    );
    q.insert("_meta", CfmlValue::strukt(driver_meta));

    CfmlValue::strukt(q)
}

fn rows_to_array(rows: Vec<serde_json::Map<String, serde_json::Value>>) -> CfmlValue {
    CfmlValue::array(
        rows.into_iter()
            .map(|row| {
                let mut rec = ValueMap::default();
                for (k, v) in row {
                    rec.insert(k, json_to_cfml(v));
                }
                CfmlValue::strukt(rec)
            })
            .collect(),
    )
}

fn rows_to_struct_by_key(
    rows: Vec<serde_json::Map<String, serde_json::Value>>,
    key: &str,
) -> CfmlValue {
    let mut out = ValueMap::default();
    for row in rows {
        let key_val = row
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .unwrap_or(serde_json::Value::Null);
        let key_str = match key_val {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        let mut rec = ValueMap::default();
        for (k, v) in row {
            rec.insert(k, json_to_cfml(v));
        }
        out.insert(key_str, CfmlValue::strukt(rec));
    }
    CfmlValue::strukt(out)
}
