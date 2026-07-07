//! cfdbinfo — datasource metadata (issue #90 Gap B).
//!
//! Result shapes follow Lucee's DBInfo.java (the canonical reference; BoxLang
//! differences are documented in docs/known-issues.md): every type returns a
//! Query except `terms` (a struct). Column names match what JDBC
//! DatabaseMetaData produces through Lucee, including Lucee's renames and
//! enrichment columns (COLUMN_DEFAULT_VALUE, IS_PRIMARYKEY, IS_FOREIGNKEY,
//! REFERENCED_PRIMARYKEY, REFERENCED_PRIMARYKEY_TABLE).
//!
//! RustCFML has no JDBC layer, so each (type × driver) cell maps to native
//! SQL — pragma functions on SQLite, information_schema on MySQL/Postgres/
//! SQL Server, sys catalogs where information_schema falls short. All SQL
//! runs through `fn_query_execute`, reusing connection pooling, per-app
//! datasource URLs and the per-driver placeholder handling (`?` everywhere;
//! the PG driver rewrites to `$n`).
//!
//! The supported `type` values are Lucee's: version, columns,
//! columns_minimal, tables, index, foreignkeys, dbnames, procedures,
//! procedure_columns, users, terms.

use cfml_common::dynamic::{CfmlQuery, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use indexmap::IndexMap;

use crate::builtins::{
    default_datasource, fn_query_execute, parse_datasource, resolve_query_datasource, DbDriver,
};

/// Entry point, registered as the `__dbinfo_impl` builtin. Takes a single
/// struct argument: the cfdbinfo attributes (already attributeCollection-
/// expanded and per-app-datasource-resolved by the VM intercept).
pub fn fn_dbinfo_impl(args: Vec<CfmlValue>) -> CfmlResult {
    let opts: ValueMap = match args.first() {
        Some(CfmlValue::Struct(s)) => s.snapshot(),
        _ => {
            return Err(CfmlError::runtime(
                "cfdbinfo: expected an attribute struct".to_string(),
            ))
        }
    };
    let attr = |k: &str| -> Option<String> {
        opts.iter()
            .find(|(kk, _)| kk.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_string())
            .filter(|s| !s.is_empty())
    };

    let info_type = attr("type")
        .ok_or_else(|| CfmlError::runtime("Missing attribute [type] on cfdbinfo".to_string()))?
        .to_lowercase();

    let ds = match attr("datasource") {
        Some(name) => resolve_query_datasource(&name)?,
        None => default_datasource().ok_or_else(|| {
            CfmlError::runtime(
                "cfdbinfo: attribute [datasource] is required when no default datasource is defined"
                    .to_string(),
            )
        })?,
    };
    let driver = parse_datasource(&ds);

    let table = attr("table");
    let pattern = attr("pattern");
    let filter = attr("filter");
    let procedure = attr("procedure");

    let require_table = |t: &Option<String>| -> Result<(String, Option<String>), CfmlError> {
        let t = t.clone().ok_or_else(|| {
            CfmlError::runtime(format!(
                "Missing attribute [table]. The type [{}] requires the attribute [table].",
                info_type
            ))
        })?;
        // `schema.table` filters by schema (Lucee parity).
        Ok(match t.split_once('.') {
            Some((schema, tbl)) => (tbl.to_string(), Some(schema.to_string())),
            None => (t, None),
        })
    };

    match info_type.as_str() {
        "version" => type_version(&ds, &driver),
        "columns" | "columns_minimal" => {
            let (tbl, schema) = require_table(&table)?;
            type_columns(
                &ds,
                &driver,
                &tbl,
                schema.as_deref(),
                pattern.as_deref(),
                info_type == "columns",
            )
        }
        "tables" => type_tables(&ds, &driver, pattern.as_deref(), filter.as_deref()),
        "index" => {
            let (tbl, schema) = require_table(&table)?;
            type_index(&ds, &driver, &tbl, schema.as_deref())
        }
        "foreignkeys" => {
            let (tbl, schema) = require_table(&table)?;
            type_foreignkeys(&ds, &driver, &tbl, schema.as_deref())
        }
        "dbnames" => type_dbnames(&ds, &driver, pattern.as_deref()),
        "procedures" => type_procedures(&ds, &driver, pattern.as_deref()),
        "procedure_columns" => {
            let proc_name = procedure.ok_or_else(|| {
                CfmlError::runtime(
                    "Missing attribute [procedure]. The type [procedure_columns] requires the attribute [procedure]."
                        .to_string(),
                )
            })?;
            type_procedure_columns(&ds, &driver, &proc_name)
        }
        "users" => type_users(&ds, &driver),
        "terms" => Ok(type_terms(&driver)),
        other => Err(CfmlError::runtime(format!(
            "invalid value [{}] for attribute [type] on cfdbinfo — supported types are \
             version, columns, columns_minimal, tables, index, foreignkeys, dbnames, \
             procedures, procedure_columns, users, terms",
            other
        ))),
    }
}

// -----------------------------------------------
// helpers
// -----------------------------------------------

/// Run SQL on the datasource through the normal queryExecute plumbing.
fn run(ds: &str, sql: &str, params: Vec<CfmlValue>) -> Result<CfmlQuery, CfmlError> {
    let mut opts = ValueMap::default();
    opts.insert("datasource".to_string(), CfmlValue::string(ds.to_string()));
    match fn_query_execute(vec![
        CfmlValue::string(sql.to_string()),
        CfmlValue::array(params),
        CfmlValue::strukt(opts),
    ])? {
        CfmlValue::Query(q) => Ok(q),
        other => Err(CfmlError::runtime(format!(
            "cfdbinfo: internal metadata query returned {} instead of a resultset",
            other.type_name()
        ))),
    }
}

/// Case-insensitive cell read from a row.
fn cell<'a>(row: &'a ValueMap, name: &str) -> Option<&'a CfmlValue> {
    row.get(name)
        .or_else(|| row.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
}

fn cell_str(row: &ValueMap, name: &str) -> String {
    cell(row, name).map(|v| v.as_string()).unwrap_or_default()
}

fn cell_int(row: &ValueMap, name: &str) -> i64 {
    match cell(row, name) {
        Some(CfmlValue::Int(i)) => *i,
        Some(CfmlValue::Double(d)) => *d as i64,
        Some(CfmlValue::Bool(b)) => *b as i64,
        Some(v) => v.as_string().trim().parse::<i64>().unwrap_or(0),
        None => 0,
    }
}

fn s(v: &str) -> CfmlValue {
    CfmlValue::string(v.to_string())
}

/// SQL LIKE matching (case-insensitive, `%` and `_` wildcards) for the
/// types where Lucee filters in Java rather than in SQL (dbnames).
fn like_match(value: &str, pattern: &str) -> bool {
    fn rec(v: &[char], p: &[char]) -> bool {
        match p.first() {
            None => v.is_empty(),
            Some('%') => (0..=v.len()).any(|i| rec(&v[i..], &p[1..])),
            Some('_') => !v.is_empty() && rec(&v[1..], &p[1..]),
            Some(c) => {
                !v.is_empty()
                    && v[0].eq_ignore_ascii_case(c)
                    && rec(&v[1..], &p[1..])
            }
        }
    }
    let v: Vec<char> = value.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    rec(&v, &p)
}

/// Lucee's missing-table error (thrown only when a table-scoped lookup
/// returned zero rows AND the table doesn't exist — a valid table with no
/// matches is not an error).
fn missing_table_error(table: &str) -> CfmlError {
    CfmlError::runtime(format!(
        "there is no table that match the following pattern [{}]",
        table
    ))
}

fn table_exists(ds: &str, driver: &DbDriver, table: &str, schema: Option<&str>) -> bool {
    let probe = match driver {
        DbDriver::Sqlite(_) => run(
            ds,
            "SELECT name FROM sqlite_master WHERE type IN ('table','view') AND lower(name) = lower(?)",
            vec![s(table)],
        ),
        DbDriver::Mysql(_) => match schema {
            Some(sc) => run(
                ds,
                "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
                vec![s(sc), s(table)],
            ),
            None => run(
                ds,
                "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
                vec![s(table)],
            ),
        },
        DbDriver::Postgres(_) => run(
            ds,
            "SELECT table_name FROM information_schema.tables WHERE table_name = lower(?) AND table_schema = COALESCE(NULLIF(lower(?), ''), current_schema())",
            vec![s(table), s(schema.unwrap_or(""))],
        ),
        DbDriver::Mssql(_) => match schema {
            Some(sc) => run(
                ds,
                "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
                vec![s(sc), s(table)],
            ),
            None => run(
                ds,
                "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = ?",
                vec![s(table)],
            ),
        },
    };
    probe.map(|q| q.row_count() > 0).unwrap_or(false)
}

/// Postgres folds unquoted identifiers to lowercase (Lucee's setCase via
/// storesLowerCaseIdentifiers); the other engines preserve what they were
/// given.
fn fold_case(driver: &DbDriver, ident: &str) -> String {
    match driver {
        DbDriver::Postgres(_) => ident.to_lowercase(),
        _ => ident.to_string(),
    }
}

// -----------------------------------------------
// type="version"
// -----------------------------------------------

fn type_version(ds: &str, driver: &DbDriver) -> CfmlResult {
    let (product, version, driver_name) = match driver {
        DbDriver::Sqlite(_) => {
            let v = run(ds, "SELECT sqlite_version() AS v", vec![])?
                .rows()
                .first()
                .map(|r| cell_str(r, "v"))
                .unwrap_or_default();
            ("SQLite".to_string(), v, "RustCFML SQLite driver")
        }
        DbDriver::Mysql(_) => {
            let v = run(ds, "SELECT VERSION() AS v", vec![])?
                .rows()
                .first()
                .map(|r| cell_str(r, "v"))
                .unwrap_or_default();
            // Report "MySQL" for the MySQL-protocol driver regardless of the
            // backend being MariaDB. This matches a Lucee/ACF deployment using
            // the MySQL JDBC connector (which reports "MySQL" even against a
            // MariaDB server), so apps that whitelist or branch on
            // `database_productname == "MySQL"` (e.g. Preside's datasource
            // validation + DB-adapter selection) work against MariaDB, which is
            // wire-compatible and uses the same driver. The MariaDB build detail
            // is still available verbatim in `database_version`.
            ("MySQL".to_string(), v, "RustCFML MySQL driver")
        }
        DbDriver::Postgres(_) => {
            // version() → "PostgreSQL 16.2 on x86_64..." / "CockroachDB CCL v23…".
            let full = run(ds, "SELECT version() AS v", vec![])?
                .rows()
                .first()
                .map(|r| cell_str(r, "v"))
                .unwrap_or_default();
            let product = if full.to_lowercase().contains("cockroachdb") {
                "CockroachDB"
            } else {
                "PostgreSQL"
            };
            let version = full.split_whitespace().nth(1).unwrap_or("").to_string();
            (product.to_string(), version, "RustCFML PostgreSQL driver")
        }
        DbDriver::Mssql(_) => {
            let v = run(
                ds,
                "SELECT CAST(SERVERPROPERTY('ProductVersion') AS NVARCHAR(128)) AS v",
                vec![],
            )?
            .rows()
            .first()
            .map(|r| cell_str(r, "v"))
            .unwrap_or_default();
            (
                "Microsoft SQL Server".to_string(),
                v,
                "RustCFML SQL Server driver",
            )
        }
    };
    let mut row = ValueMap::default();
    row.insert("database_productname".to_string(), s(&product));
    row.insert("database_version".to_string(), s(&version));
    row.insert("driver_name".to_string(), s(driver_name));
    row.insert(
        "driver_version".to_string(),
        s(env!("CARGO_PKG_VERSION")),
    );
    row.insert("jdbc_major_version".to_string(), CfmlValue::Int(0));
    row.insert("jdbc_minor_version".to_string(), CfmlValue::Int(0));
    Ok(CfmlValue::Query(CfmlQuery::from_parts(
        vec![
            "database_productname".to_string(),
            "database_version".to_string(),
            "driver_name".to_string(),
            "driver_version".to_string(),
            "jdbc_major_version".to_string(),
            "jdbc_minor_version".to_string(),
        ],
        vec![row],
    )))
}

// -----------------------------------------------
// type="columns" / "columns_minimal"
// -----------------------------------------------

const COLUMNS_COLUMNS: &[&str] = &[
    "TABLE_CAT",
    "TABLE_SCHEM",
    "TABLE_NAME",
    "COLUMN_NAME",
    "DATA_TYPE",
    "TYPE_NAME",
    "COLUMN_SIZE",
    "BUFFER_LENGTH",
    "DECIMAL_DIGITS",
    "NUM_PREC_RADIX",
    "NULLABLE",
    "REMARKS",
    "COLUMN_DEFAULT_VALUE",
    "SQL_DATA_TYPE",
    "SQL_DATETIME_SUB",
    "CHAR_OCTET_LENGTH",
    "ORDINAL_POSITION",
    "IS_NULLABLE",
];

const COLUMNS_ENRICHMENT: &[&str] = &[
    "IS_PRIMARYKEY",
    "IS_AUTOINCREMENT",
    "IS_FOREIGNKEY",
    "REFERENCED_PRIMARYKEY",
    "REFERENCED_PRIMARYKEY_TABLE",
];

/// One source column, driver-neutral, before shaping into the Lucee result.
struct ColInfo {
    table_cat: String,
    table_schem: String,
    table_name: String,
    column_name: String,
    type_name: String,
    column_size: i64,
    decimal_digits: i64,
    nullable: bool,
    default_value: Option<String>,
    ordinal: i64,
    is_pk: bool,
    is_autoincrement: bool,
}

fn type_columns(
    ds: &str,
    driver: &DbDriver,
    table: &str,
    schema: Option<&str>,
    pattern: Option<&str>,
    enrich: bool,
) -> CfmlResult {
    let table = fold_case(driver, table);
    let schema_owned = schema.map(|sc| fold_case(driver, sc));
    let schema = schema_owned.as_deref();

    let (cols, fk_map) = match driver {
        DbDriver::Sqlite(_) => {
            let base = run(
                ds,
                "SELECT cid, name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?)",
                vec![s(&table)],
            )?;
            let mut cols = Vec::new();
            for row in base.rows() {
                let decl = cell_str(&row, "type");
                // "VARCHAR(50)" → TYPE_NAME "VARCHAR", COLUMN_SIZE 50.
                let (type_name, size) = match decl.split_once('(') {
                    Some((head, tail)) => (
                        head.trim().to_string(),
                        tail.trim_end_matches(')')
                            .split(',')
                            .next()
                            .and_then(|n| n.trim().parse::<i64>().ok())
                            .unwrap_or(2_000_000_000),
                    ),
                    None => (decl.trim().to_string(), 2_000_000_000),
                };
                let dflt = match cell(&row, "dflt_value") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                // SQLite auto-increment is the INTEGER PRIMARY KEY rowid alias
                // (with or without the AUTOINCREMENT keyword). Computed before
                // `type_name` is moved into the struct.
                let is_autoincrement =
                    cell_int(&row, "pk") > 0 && type_name.eq_ignore_ascii_case("INTEGER");
                cols.push(ColInfo {
                    table_cat: "main".to_string(),
                    table_schem: String::new(),
                    table_name: table.clone(),
                    column_name: cell_str(&row, "name"),
                    type_name,
                    column_size: size,
                    decimal_digits: 0,
                    nullable: cell_int(&row, "notnull") == 0,
                    default_value: dflt,
                    ordinal: cell_int(&row, "cid") + 1,
                    is_pk: cell_int(&row, "pk") > 0,
                    is_autoincrement,
                });
            }
            let mut fk_map: IndexMap<String, (String, String)> = IndexMap::new();
            if enrich {
                let fks = run(
                    ds,
                    "SELECT \"from\", \"to\", \"table\" FROM pragma_foreign_key_list(?)",
                    vec![s(&table)],
                )?;
                for row in fks.rows() {
                    fk_map.insert(
                        cell_str(&row, "from").to_lowercase(),
                        (cell_str(&row, "to"), cell_str(&row, "table")),
                    );
                }
            }
            (cols, fk_map)
        }
        DbDriver::Mysql(_) => {
            let (sql, params) = match schema {
                Some(sc) => (
                    "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, \
                     CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, NUMERIC_SCALE, IS_NULLABLE, \
                     COLUMN_DEFAULT, ORDINAL_POSITION, COLUMN_KEY, EXTRA \
                     FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    vec![s(sc), s(&table)],
                ),
                None => (
                    "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, \
                     CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, NUMERIC_SCALE, IS_NULLABLE, \
                     COLUMN_DEFAULT, ORDINAL_POSITION, COLUMN_KEY, EXTRA \
                     FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    vec![s(&table)],
                ),
            };
            let base = run(ds, sql, params)?;
            let mut cols = Vec::new();
            for row in base.rows() {
                // COLUMN_TYPE "int(10) unsigned" → TYPE_NAME "int unsigned"
                // (matches JDBC's "INT UNSIGNED" shape Wheels parses as a
                // space-delimited list).
                let column_type = cell_str(&row, "COLUMN_TYPE");
                let mut type_name = String::with_capacity(column_type.len());
                let mut in_parens = false;
                for c in column_type.chars() {
                    match c {
                        '(' => in_parens = true,
                        ')' => in_parens = false,
                        _ if !in_parens => type_name.push(c),
                        _ => {}
                    }
                }
                let type_name = type_name.split_whitespace().collect::<Vec<_>>().join(" ");
                let size = match cell(&row, "CHARACTER_MAXIMUM_LENGTH") {
                    Some(CfmlValue::Null) | None => cell_int(&row, "NUMERIC_PRECISION"),
                    Some(v) => v.as_string().parse::<i64>().unwrap_or(0),
                };
                let dflt = match cell(&row, "COLUMN_DEFAULT") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                cols.push(ColInfo {
                    table_cat: cell_str(&row, "TABLE_SCHEMA"),
                    table_schem: String::new(),
                    table_name: cell_str(&row, "TABLE_NAME"),
                    column_name: cell_str(&row, "COLUMN_NAME"),
                    type_name,
                    column_size: size,
                    decimal_digits: cell_int(&row, "NUMERIC_SCALE"),
                    nullable: cell_str(&row, "IS_NULLABLE").eq_ignore_ascii_case("YES"),
                    default_value: dflt,
                    ordinal: cell_int(&row, "ORDINAL_POSITION"),
                    is_pk: cell_str(&row, "COLUMN_KEY").eq_ignore_ascii_case("PRI"),
                    // MySQL/MariaDB flags auto-increment in the EXTRA column
                    // ("auto_increment"), mirroring JDBC's IS_AUTOINCREMENT.
                    // Preside's schema synchronizer reads column.is_autoincrement
                    // (GitHub #254).
                    is_autoincrement: cell_str(&row, "EXTRA")
                        .to_lowercase()
                        .contains("auto_increment"),
                });
            }
            let mut fk_map: IndexMap<String, (String, String)> = IndexMap::new();
            if enrich {
                let fks = run(
                    ds,
                    "SELECT COLUMN_NAME, REFERENCED_COLUMN_NAME, REFERENCED_TABLE_NAME \
                     FROM information_schema.KEY_COLUMN_USAGE \
                     WHERE TABLE_SCHEMA = COALESCE(NULLIF(?, ''), DATABASE()) AND TABLE_NAME = ? \
                     AND REFERENCED_TABLE_NAME IS NOT NULL",
                    vec![s(schema.unwrap_or("")), s(&table)],
                )?;
                for row in fks.rows() {
                    fk_map.insert(
                        cell_str(&row, "COLUMN_NAME").to_lowercase(),
                        (
                            cell_str(&row, "REFERENCED_COLUMN_NAME"),
                            cell_str(&row, "REFERENCED_TABLE_NAME"),
                        ),
                    );
                }
            }
            (cols, fk_map)
        }
        DbDriver::Postgres(_) => {
            let base = run(
                ds,
                "SELECT table_catalog, table_schema, table_name, column_name, udt_name, \
                 character_maximum_length, numeric_precision, numeric_scale, is_nullable, \
                 column_default, ordinal_position \
                 FROM information_schema.columns \
                 WHERE table_schema = COALESCE(NULLIF(?, ''), current_schema()) AND table_name = ? \
                 ORDER BY ordinal_position",
                vec![s(schema.unwrap_or("")), s(&table)],
            )?;
            let pk_rows = run(
                ds,
                "SELECT kcu.column_name FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
                 WHERE tc.constraint_type = 'PRIMARY KEY' \
                   AND tc.table_schema = COALESCE(NULLIF(?, ''), current_schema()) AND tc.table_name = ?",
                vec![s(schema.unwrap_or("")), s(&table)],
            )?;
            let pk_set: Vec<String> = pk_rows
                .rows()
                .iter()
                .map(|r| cell_str(r, "column_name").to_lowercase())
                .collect();
            let mut cols = Vec::new();
            for row in base.rows() {
                let size = match cell(&row, "character_maximum_length") {
                    Some(CfmlValue::Null) | None => cell_int(&row, "numeric_precision"),
                    Some(v) => v.as_string().parse::<i64>().unwrap_or(0),
                };
                let dflt = match cell(&row, "column_default") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                let name = cell_str(&row, "column_name");
                cols.push(ColInfo {
                    table_cat: cell_str(&row, "table_catalog"),
                    table_schem: cell_str(&row, "table_schema"),
                    table_name: cell_str(&row, "table_name"),
                    is_pk: pk_set.contains(&name.to_lowercase()),
                    column_name: name,
                    type_name: cell_str(&row, "udt_name"),
                    column_size: size,
                    decimal_digits: cell_int(&row, "numeric_scale"),
                    nullable: cell_str(&row, "is_nullable").eq_ignore_ascii_case("YES"),
                    // Postgres serial/bigserial columns default to nextval(seq);
                    // that (or an IDENTITY default) is the auto-increment marker.
                    is_autoincrement: dflt
                        .as_deref()
                        .map(|d| d.to_lowercase().contains("nextval"))
                        .unwrap_or(false),
                    default_value: dflt,
                    ordinal: cell_int(&row, "ordinal_position"),
                });
            }
            let mut fk_map: IndexMap<String, (String, String)> = IndexMap::new();
            if enrich {
                let fks = run(
                    ds,
                    "SELECT kcu.column_name AS fkcol, ccu.column_name AS pkcol, ccu.table_name AS pktable \
                     FROM information_schema.table_constraints tc \
                     JOIN information_schema.key_column_usage kcu \
                       ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
                     JOIN information_schema.referential_constraints rc \
                       ON tc.constraint_name = rc.constraint_name \
                     JOIN information_schema.constraint_column_usage ccu \
                       ON rc.unique_constraint_name = ccu.constraint_name \
                     WHERE tc.constraint_type = 'FOREIGN KEY' \
                       AND tc.table_schema = COALESCE(NULLIF(?, ''), current_schema()) AND tc.table_name = ?",
                    vec![s(schema.unwrap_or("")), s(&table)],
                )?;
                for row in fks.rows() {
                    fk_map.insert(
                        cell_str(&row, "fkcol").to_lowercase(),
                        (cell_str(&row, "pkcol"), cell_str(&row, "pktable")),
                    );
                }
            }
            (cols, fk_map)
        }
        DbDriver::Mssql(_) => {
            let (sql, params) = match schema {
                Some(sc) => (
                    "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, DATA_TYPE, \
                     CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, NUMERIC_SCALE, IS_NULLABLE, \
                     COLUMN_DEFAULT, ORDINAL_POSITION \
                     FROM INFORMATION_SCHEMA.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    vec![s(sc), s(&table)],
                ),
                None => (
                    "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, DATA_TYPE, \
                     CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, NUMERIC_SCALE, IS_NULLABLE, \
                     COLUMN_DEFAULT, ORDINAL_POSITION \
                     FROM INFORMATION_SCHEMA.COLUMNS \
                     WHERE TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    vec![s(&table)],
                ),
            };
            let base = run(ds, sql, params)?;
            let pk_rows = run(
                ds,
                "SELECT kcu.COLUMN_NAME FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
                 JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE kcu \
                   ON tc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
                 WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' AND tc.TABLE_NAME = ?",
                vec![s(&table)],
            )?;
            let pk_set: Vec<String> = pk_rows
                .rows()
                .iter()
                .map(|r| cell_str(r, "COLUMN_NAME").to_lowercase())
                .collect();
            let mut cols = Vec::new();
            for row in base.rows() {
                let size = match cell(&row, "CHARACTER_MAXIMUM_LENGTH") {
                    Some(CfmlValue::Null) | None => cell_int(&row, "NUMERIC_PRECISION"),
                    Some(v) => v.as_string().parse::<i64>().unwrap_or(0),
                };
                let dflt = match cell(&row, "COLUMN_DEFAULT") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                let name = cell_str(&row, "COLUMN_NAME");
                cols.push(ColInfo {
                    table_cat: cell_str(&row, "TABLE_CATALOG"),
                    table_schem: cell_str(&row, "TABLE_SCHEMA"),
                    table_name: cell_str(&row, "TABLE_NAME"),
                    is_pk: pk_set.contains(&name.to_lowercase()),
                    column_name: name,
                    type_name: cell_str(&row, "DATA_TYPE"),
                    column_size: size,
                    decimal_digits: cell_int(&row, "NUMERIC_SCALE"),
                    nullable: cell_str(&row, "IS_NULLABLE").eq_ignore_ascii_case("YES"),
                    default_value: dflt,
                    ordinal: cell_int(&row, "ORDINAL_POSITION"),
                    // SQL Server IDENTITY is not exposed via INFORMATION_SCHEMA
                    // (it needs COLUMNPROPERTY(...,'IsIdentity')); left false as a
                    // best-effort default (the reported case is MySQL, GitHub #254).
                    is_autoincrement: false,
                });
            }
            let mut fk_map: IndexMap<String, (String, String)> = IndexMap::new();
            if enrich {
                let fks = run(
                    ds,
                    "SELECT kcu.COLUMN_NAME AS fkcol, ccu.COLUMN_NAME AS pkcol, ccu.TABLE_NAME AS pktable \
                     FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
                     JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE kcu \
                       ON tc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME \
                     JOIN INFORMATION_SCHEMA.REFERENTIAL_CONSTRAINTS rc \
                       ON tc.CONSTRAINT_NAME = rc.CONSTRAINT_NAME \
                     JOIN INFORMATION_SCHEMA.CONSTRAINT_COLUMN_USAGE ccu \
                       ON rc.UNIQUE_CONSTRAINT_NAME = ccu.CONSTRAINT_NAME \
                     WHERE tc.CONSTRAINT_TYPE = 'FOREIGN KEY' AND tc.TABLE_NAME = ?",
                    vec![s(&table)],
                )?;
                for row in fks.rows() {
                    fk_map.insert(
                        cell_str(&row, "fkcol").to_lowercase(),
                        (cell_str(&row, "pkcol"), cell_str(&row, "pktable")),
                    );
                }
            }
            (cols, fk_map)
        }
    };

    if cols.is_empty() && !table_exists(ds, driver, &table, schema) {
        return Err(missing_table_error(&table));
    }

    let mut out_columns: Vec<String> = COLUMNS_COLUMNS.iter().map(|c| c.to_string()).collect();
    if enrich {
        out_columns.extend(COLUMNS_ENRICHMENT.iter().map(|c| c.to_string()));
    }
    let mut rows = Vec::with_capacity(cols.len());
    for c in cols {
        // Optional column-name pattern filter (Lucee's `pattern` attribute).
        if let Some(p) = pattern {
            if !like_match(&c.column_name, p) {
                continue;
            }
        }
        let mut row = ValueMap::default();
        row.insert("TABLE_CAT".to_string(), s(&c.table_cat));
        row.insert("TABLE_SCHEM".to_string(), s(&c.table_schem));
        row.insert("TABLE_NAME".to_string(), s(&c.table_name));
        row.insert("COLUMN_NAME".to_string(), s(&c.column_name));
        row.insert("DATA_TYPE".to_string(), CfmlValue::Int(0));
        row.insert("TYPE_NAME".to_string(), s(&c.type_name));
        row.insert("COLUMN_SIZE".to_string(), CfmlValue::Int(c.column_size));
        row.insert("BUFFER_LENGTH".to_string(), CfmlValue::Int(0));
        row.insert(
            "DECIMAL_DIGITS".to_string(),
            CfmlValue::Int(c.decimal_digits),
        );
        row.insert("NUM_PREC_RADIX".to_string(), CfmlValue::Int(10));
        row.insert(
            "NULLABLE".to_string(),
            CfmlValue::Int(if c.nullable { 1 } else { 0 }),
        );
        row.insert("REMARKS".to_string(), s(""));
        row.insert(
            "COLUMN_DEFAULT_VALUE".to_string(),
            match &c.default_value {
                Some(d) => s(d),
                None => CfmlValue::Null,
            },
        );
        row.insert("SQL_DATA_TYPE".to_string(), CfmlValue::Int(0));
        row.insert("SQL_DATETIME_SUB".to_string(), CfmlValue::Int(0));
        row.insert(
            "CHAR_OCTET_LENGTH".to_string(),
            CfmlValue::Int(c.column_size),
        );
        row.insert("ORDINAL_POSITION".to_string(), CfmlValue::Int(c.ordinal));
        row.insert(
            "IS_NULLABLE".to_string(),
            s(if c.nullable { "YES" } else { "NO" }),
        );
        if enrich {
            row.insert(
                "IS_PRIMARYKEY".to_string(),
                s(if c.is_pk { "YES" } else { "NO" }),
            );
            row.insert(
                "IS_AUTOINCREMENT".to_string(),
                s(if c.is_autoincrement { "YES" } else { "NO" }),
            );
            match fk_map.get(&c.column_name.to_lowercase()) {
                Some((pkcol, pktable)) => {
                    row.insert("IS_FOREIGNKEY".to_string(), s("YES"));
                    row.insert("REFERENCED_PRIMARYKEY".to_string(), s(pkcol));
                    row.insert("REFERENCED_PRIMARYKEY_TABLE".to_string(), s(pktable));
                }
                None => {
                    row.insert("IS_FOREIGNKEY".to_string(), s("NO"));
                    row.insert("REFERENCED_PRIMARYKEY".to_string(), s("N/A"));
                    row.insert("REFERENCED_PRIMARYKEY_TABLE".to_string(), s("N/A"));
                }
            }
        }
        rows.push(row);
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(out_columns, rows)))
}

// -----------------------------------------------
// type="tables"
// -----------------------------------------------

const TABLE_TYPE_FILTERS: &[&str] = &[
    "TABLE",
    "VIEW",
    "SYSTEM TABLE",
    "GLOBAL TEMPORARY",
    "LOCAL TEMPORARY",
    "ALIAS",
    "SYNONYM",
];

fn type_tables(
    ds: &str,
    driver: &DbDriver,
    pattern: Option<&str>,
    filter: Option<&str>,
) -> CfmlResult {
    let filter_upper = filter.map(|f| f.to_uppercase());
    if let Some(ref f) = filter_upper {
        if !TABLE_TYPE_FILTERS.contains(&f.as_str()) {
            return Err(CfmlError::runtime(format!(
                "Invalid [dbinfo] type=table filter [{}]. Supported table types are {:?}.",
                f, TABLE_TYPE_FILTERS
            )));
        }
    }
    // (catalog, schema, name, raw_type)
    let raw: Vec<(String, String, String, String)> = match driver {
        DbDriver::Sqlite(_) => run(
            ds,
            "SELECT name, type FROM sqlite_master WHERE type IN ('table','view') \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                "main".to_string(),
                String::new(),
                cell_str(r, "name"),
                cell_str(r, "type").to_uppercase(),
            )
        })
        .collect(),
        DbDriver::Mysql(_) => run(
            ds,
            "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "TABLE_SCHEMA"),
                String::new(),
                cell_str(r, "TABLE_NAME"),
                cell_str(r, "TABLE_TYPE"),
            )
        })
        .collect(),
        DbDriver::Postgres(_) => run(
            ds,
            "SELECT table_catalog, table_schema, table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_name",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "table_catalog"),
                cell_str(r, "table_schema"),
                cell_str(r, "table_name"),
                cell_str(r, "table_type"),
            )
        })
        .collect(),
        DbDriver::Mssql(_) => run(
            ds,
            "SELECT TABLE_CATALOG, TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE \
             FROM INFORMATION_SCHEMA.TABLES ORDER BY TABLE_NAME",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "TABLE_CATALOG"),
                cell_str(r, "TABLE_SCHEMA"),
                cell_str(r, "TABLE_NAME"),
                cell_str(r, "TABLE_TYPE"),
            )
        })
        .collect(),
    };

    let columns = vec![
        "TABLE_CAT".to_string(),
        "TABLE_SCHEM".to_string(),
        "TABLE_NAME".to_string(),
        "TABLE_TYPE".to_string(),
        "REMARKS".to_string(),
    ];
    let mut rows = Vec::new();
    for (cat, schem, name, raw_type) in raw {
        // JDBC table-type vocabulary: "BASE TABLE" (information_schema)
        // reports as "TABLE".
        let table_type = match raw_type.to_uppercase().as_str() {
            "BASE TABLE" | "TABLE" => "TABLE".to_string(),
            "VIEW" => "VIEW".to_string(),
            other => other.to_string(),
        };
        if let Some(ref f) = filter_upper {
            if &table_type != f {
                continue;
            }
        }
        if let Some(p) = pattern {
            if !like_match(&name, p) {
                continue;
            }
        }
        let mut row = ValueMap::default();
        row.insert("TABLE_CAT".to_string(), s(&cat));
        row.insert("TABLE_SCHEM".to_string(), s(&schem));
        row.insert("TABLE_NAME".to_string(), s(&name));
        row.insert("TABLE_TYPE".to_string(), s(&table_type));
        row.insert("REMARKS".to_string(), s(""));
        rows.push(row);
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

// -----------------------------------------------
// type="index"
// -----------------------------------------------

fn type_index(ds: &str, driver: &DbDriver, table: &str, schema: Option<&str>) -> CfmlResult {
    let table = fold_case(driver, table);
    let columns = vec![
        "TABLE_CAT".to_string(),
        "TABLE_SCHEM".to_string(),
        "TABLE_NAME".to_string(),
        "NON_UNIQUE".to_string(),
        "INDEX_QUALIFIER".to_string(),
        "INDEX_NAME".to_string(),
        "TYPE".to_string(),
        "ORDINAL_POSITION".to_string(),
        "COLUMN_NAME".to_string(),
        "ASC_OR_DESC".to_string(),
        "CARDINALITY".to_string(),
        "PAGES".to_string(),
        "FILTER_CONDITION".to_string(),
    ];
    let mut rows = Vec::new();
    let mut push_row = |table_name: &str,
                        non_unique: i64,
                        index_name: &str,
                        index_type: &str,
                        ordinal: i64,
                        column_name: &str,
                        asc: &str,
                        cardinality: i64| {
        let mut row = ValueMap::default();
        row.insert("TABLE_CAT".to_string(), CfmlValue::Null);
        row.insert("TABLE_SCHEM".to_string(), CfmlValue::Null);
        row.insert("TABLE_NAME".to_string(), s(table_name));
        row.insert("NON_UNIQUE".to_string(), CfmlValue::Int(non_unique));
        row.insert("INDEX_QUALIFIER".to_string(), CfmlValue::Null);
        row.insert("INDEX_NAME".to_string(), s(index_name));
        row.insert("TYPE".to_string(), s(index_type));
        row.insert("ORDINAL_POSITION".to_string(), CfmlValue::Int(ordinal));
        row.insert("COLUMN_NAME".to_string(), s(column_name));
        row.insert("ASC_OR_DESC".to_string(), s(asc));
        row.insert("CARDINALITY".to_string(), CfmlValue::Int(cardinality));
        row.insert("PAGES".to_string(), CfmlValue::Int(0));
        row.insert("FILTER_CONDITION".to_string(), s(""));
        rows.push(row);
    };

    match driver {
        DbDriver::Sqlite(_) => {
            // Named indexes (incl. UNIQUE auto-indexes) + PRIMARY KEY rows —
            // the same shape Wheels' SQLite shim emits.
            let q = run(
                ds,
                "SELECT il.name AS index_name, il.\"unique\" AS uniq, ii.seqno + 1 AS ord, \
                 ii.name AS column_name \
                 FROM pragma_index_list(?1) il JOIN pragma_index_info(il.name) ii \
                 ORDER BY il.name, ii.seqno",
                vec![s(&table)],
            )?;
            for r in q.rows() {
                push_row(
                    &table,
                    if cell_int(&r, "uniq") == 0 { 1 } else { 0 },
                    &cell_str(&r, "index_name"),
                    "Other Index",
                    cell_int(&r, "ord"),
                    &cell_str(&r, "column_name"),
                    "A",
                    0,
                );
            }
            let pks = run(
                ds,
                "SELECT name, pk FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk",
                vec![s(&table)],
            )?;
            for r in pks.rows() {
                push_row(
                    &table,
                    0,
                    "PRIMARY",
                    "Primary Key",
                    cell_int(&r, "pk"),
                    &cell_str(&r, "name"),
                    "A",
                    0,
                );
            }
        }
        DbDriver::Mysql(_) => {
            let q = run(
                ds,
                "SELECT TABLE_NAME, NON_UNIQUE, INDEX_NAME, SEQ_IN_INDEX, COLUMN_NAME, \
                 COLLATION, CARDINALITY FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = COALESCE(NULLIF(?, ''), DATABASE()) AND TABLE_NAME = ? \
                 ORDER BY INDEX_NAME, SEQ_IN_INDEX",
                vec![s(schema.unwrap_or("")), s(&table)],
            )?;
            for r in q.rows() {
                push_row(
                    &cell_str(&r, "TABLE_NAME"),
                    cell_int(&r, "NON_UNIQUE"),
                    &cell_str(&r, "INDEX_NAME"),
                    "Other Index",
                    cell_int(&r, "SEQ_IN_INDEX"),
                    &cell_str(&r, "COLUMN_NAME"),
                    if cell_str(&r, "COLLATION") == "D" { "D" } else { "A" },
                    cell_int(&r, "CARDINALITY"),
                );
            }
        }
        DbDriver::Postgres(_) => {
            let q = run(
                ds,
                "SELECT t.relname AS table_name, \
                 CASE WHEN ix.indisunique THEN 0 ELSE 1 END AS non_unique, \
                 i.relname AS index_name, a.ord AS ord, att.attname AS column_name \
                 FROM pg_class t \
                 JOIN pg_namespace n ON n.oid = t.relnamespace \
                 JOIN pg_index ix ON t.oid = ix.indrelid \
                 JOIN pg_class i ON i.oid = ix.indexrelid \
                 CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS a(attnum, ord) \
                 JOIN pg_attribute att ON att.attrelid = t.oid AND att.attnum = a.attnum \
                 WHERE t.relname = ? \
                   AND n.nspname = COALESCE(NULLIF(?, ''), current_schema()) \
                 ORDER BY i.relname, a.ord",
                vec![s(&table), s(schema.unwrap_or(""))],
            )?;
            for r in q.rows() {
                push_row(
                    &cell_str(&r, "table_name"),
                    cell_int(&r, "non_unique"),
                    &cell_str(&r, "index_name"),
                    "Other Index",
                    cell_int(&r, "ord"),
                    &cell_str(&r, "column_name"),
                    "A",
                    0,
                );
            }
        }
        DbDriver::Mssql(_) => {
            // Reference semantics: the sys.indexes query Wheels uses for its
            // BoxLang-MSSQL workaround (vendor/wheels/Global.cfc).
            let q = run(
                ds,
                "SELECT t.name AS table_name, \
                 CAST(CASE WHEN i.is_unique = 0 THEN 1 ELSE 0 END AS INT) AS non_unique, \
                 i.name AS index_name, \
                 CASE WHEN i.type = 1 THEN 'Clustered Index' ELSE 'Other Index' END AS index_type, \
                 CAST(ic.key_ordinal AS INT) AS ord, c.name AS column_name, \
                 CASE WHEN ic.is_descending_key = 0 THEN 'A' ELSE 'D' END AS asc_or_desc \
                 FROM sys.indexes i \
                 INNER JOIN sys.objects t ON i.object_id = t.object_id \
                 INNER JOIN sys.index_columns ic \
                   ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
                 INNER JOIN sys.columns c \
                   ON ic.object_id = c.object_id AND ic.column_id = c.column_id \
                 WHERE t.name = ? AND t.type = 'U' \
                   AND i.type_desc IN ('CLUSTERED','NONCLUSTERED') \
                 ORDER BY i.name, ic.key_ordinal",
                vec![s(&table)],
            )?;
            for r in q.rows() {
                push_row(
                    &cell_str(&r, "table_name"),
                    cell_int(&r, "non_unique"),
                    &cell_str(&r, "index_name"),
                    &cell_str(&r, "index_type"),
                    cell_int(&r, "ord"),
                    &cell_str(&r, "column_name"),
                    &cell_str(&r, "asc_or_desc"),
                    0,
                );
            }
        }
    }

    if rows.is_empty() && !table_exists(ds, driver, &table, schema) {
        return Err(missing_table_error(&table));
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

// -----------------------------------------------
// type="foreignkeys"  (exported keys: FKs in other tables referencing TABLE)
// -----------------------------------------------

// Map a textual referential action (as reported by information_schema /
// SQLite pragma) to the JDBC `DatabaseMetaData` numeric rule code that Lucee's
// `dbinfo type="foreignkeys"` surfaces in UPDATE_RULE / DELETE_RULE. Callers
// (e.g. Preside's dbSync) key a struct on these integers ("0"=cascade,
// "2"=set null), so returning the textual form silently mis-classified every
// rule. importedKeyCascade=0, importedKeyRestrict=1, importedKeySetNull=2,
// importedKeyNoAction=3, importedKeySetDefault=4.
fn fk_rule_code(rule: &str) -> i64 {
    match rule.trim().to_ascii_uppercase().as_str() {
        "CASCADE" => 0,
        "RESTRICT" => 1,
        "SET NULL" | "SETNULL" => 2,
        "SET DEFAULT" | "SETDEFAULT" => 4,
        // "NO ACTION" and anything unrecognised behave as no-action.
        _ => 3,
    }
}

fn type_foreignkeys(ds: &str, driver: &DbDriver, table: &str, schema: Option<&str>) -> CfmlResult {
    let table = fold_case(driver, table);
    let columns = vec![
        "PKTABLE_CAT".to_string(),
        "PKTABLE_SCHEM".to_string(),
        "PKTABLE_NAME".to_string(),
        "PKCOLUMN_NAME".to_string(),
        "FKTABLE_CAT".to_string(),
        "FKTABLE_SCHEM".to_string(),
        "FKTABLE_NAME".to_string(),
        "FKCOLUMN_NAME".to_string(),
        "KEY_SEQ".to_string(),
        "UPDATE_RULE".to_string(),
        "DELETE_RULE".to_string(),
        "FK_NAME".to_string(),
        "PK_NAME".to_string(),
        "DEFERRABILITY".to_string(),
    ];
    // (pkcolumn, fktable, fkcolumn, key_seq, update_rule, delete_rule, fk_name)
    let raw: Vec<(String, String, String, i64, String, String, String)> = match driver {
        DbDriver::Sqlite(_) => run(
            ds,
            "SELECT m.name AS fktable, fk.\"from\" AS fkcol, COALESCE(fk.\"to\", '') AS pkcol, \
             fk.seq + 1 AS key_seq, fk.on_update, fk.on_delete \
             FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) fk \
             WHERE m.type = 'table' AND lower(fk.\"table\") = lower(?1) \
             ORDER BY m.name, fk.id, fk.seq",
            vec![s(&table)],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "pkcol"),
                cell_str(r, "fktable"),
                cell_str(r, "fkcol"),
                cell_int(r, "key_seq"),
                cell_str(r, "on_update"),
                cell_str(r, "on_delete"),
                String::new(),
            )
        })
        .collect(),
        DbDriver::Mysql(_) => run(
            ds,
            "SELECT kcu.REFERENCED_COLUMN_NAME AS pkcol, kcu.TABLE_NAME AS fktable, \
             kcu.COLUMN_NAME AS fkcol, kcu.ORDINAL_POSITION AS key_seq, \
             rc.UPDATE_RULE AS update_rule, rc.DELETE_RULE AS delete_rule, \
             kcu.CONSTRAINT_NAME AS fk_name \
             FROM information_schema.KEY_COLUMN_USAGE kcu \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS rc \
               ON kcu.CONSTRAINT_NAME = rc.CONSTRAINT_NAME \
               AND kcu.CONSTRAINT_SCHEMA = rc.CONSTRAINT_SCHEMA \
             WHERE kcu.TABLE_SCHEMA = COALESCE(NULLIF(?, ''), DATABASE()) \
               AND kcu.REFERENCED_TABLE_NAME = ? \
             ORDER BY kcu.TABLE_NAME, kcu.ORDINAL_POSITION",
            vec![s(schema.unwrap_or("")), s(&table)],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "pkcol"),
                cell_str(r, "fktable"),
                cell_str(r, "fkcol"),
                cell_int(r, "key_seq"),
                cell_str(r, "update_rule"),
                cell_str(r, "delete_rule"),
                cell_str(r, "fk_name"),
            )
        })
        .collect(),
        DbDriver::Postgres(_) | DbDriver::Mssql(_) => {
            // Both speak standard information_schema.
            let sql = "SELECT ccu.column_name AS pkcol, tc.table_name AS fktable, \
                 kcu.column_name AS fkcol, kcu.ordinal_position AS key_seq, \
                 rc.update_rule AS update_rule, rc.delete_rule AS delete_rule, \
                 tc.constraint_name AS fk_name \
                 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                 JOIN information_schema.referential_constraints rc \
                   ON tc.constraint_name = rc.constraint_name \
                 JOIN information_schema.constraint_column_usage ccu \
                   ON rc.unique_constraint_name = ccu.constraint_name \
                 WHERE tc.constraint_type = 'FOREIGN KEY' AND ccu.table_name = ? \
                 ORDER BY tc.table_name, kcu.ordinal_position";
            run(ds, sql, vec![s(&table)])?
                .rows()
                .iter()
                .map(|r| {
                    (
                        cell_str(r, "pkcol"),
                        cell_str(r, "fktable"),
                        cell_str(r, "fkcol"),
                        cell_int(r, "key_seq"),
                        cell_str(r, "update_rule"),
                        cell_str(r, "delete_rule"),
                        cell_str(r, "fk_name"),
                    )
                })
                .collect()
        }
    };

    if raw.is_empty() && !table_exists(ds, driver, &table, schema) {
        return Err(missing_table_error(&table));
    }

    let mut rows = Vec::new();
    for (pkcol, fktable, fkcol, key_seq, upd, del, fk_name) in raw {
        let mut row = ValueMap::default();
        row.insert("PKTABLE_CAT".to_string(), CfmlValue::Null);
        row.insert("PKTABLE_SCHEM".to_string(), CfmlValue::Null);
        row.insert("PKTABLE_NAME".to_string(), s(&table));
        row.insert("PKCOLUMN_NAME".to_string(), s(&pkcol));
        row.insert("FKTABLE_CAT".to_string(), CfmlValue::Null);
        row.insert("FKTABLE_SCHEM".to_string(), CfmlValue::Null);
        row.insert("FKTABLE_NAME".to_string(), s(&fktable));
        row.insert("FKCOLUMN_NAME".to_string(), s(&fkcol));
        row.insert("KEY_SEQ".to_string(), CfmlValue::Int(key_seq));
        row.insert("UPDATE_RULE".to_string(), CfmlValue::Int(fk_rule_code(&upd)));
        row.insert("DELETE_RULE".to_string(), CfmlValue::Int(fk_rule_code(&del)));
        row.insert("FK_NAME".to_string(), s(&fk_name));
        row.insert("PK_NAME".to_string(), s(""));
        row.insert("DEFERRABILITY".to_string(), CfmlValue::Int(7));
        rows.push(row);
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

// -----------------------------------------------
// type="dbnames"
// -----------------------------------------------

fn type_dbnames(ds: &str, driver: &DbDriver, pattern: Option<&str>) -> CfmlResult {
    // (database_name, type)
    let raw: Vec<(String, String)> = match driver {
        DbDriver::Sqlite(_) => run(ds, "SELECT name FROM pragma_database_list", vec![])?
            .rows()
            .iter()
            .map(|r| (cell_str(r, "name"), "CATALOG".to_string()))
            .collect(),
        DbDriver::Mysql(_) => run(
            ds,
            "SELECT SCHEMA_NAME AS n FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| (cell_str(r, "n"), "CATALOG".to_string()))
        .collect(),
        DbDriver::Postgres(_) => {
            let mut v: Vec<(String, String)> = run(
                ds,
                "SELECT datname AS n FROM pg_database WHERE NOT datistemplate ORDER BY datname",
                vec![],
            )?
            .rows()
            .iter()
            .map(|r| (cell_str(r, "n"), "CATALOG".to_string()))
            .collect();
            v.extend(
                run(
                    ds,
                    "SELECT nspname AS n FROM pg_namespace WHERE nspname NOT LIKE 'pg_%' ORDER BY nspname",
                    vec![],
                )?
                .rows()
                .iter()
                .map(|r| (cell_str(r, "n"), "SCHEMA".to_string())),
            );
            v
        }
        DbDriver::Mssql(_) => {
            let mut v: Vec<(String, String)> = run(
                ds,
                "SELECT name AS n FROM sys.databases ORDER BY name",
                vec![],
            )?
            .rows()
            .iter()
            .map(|r| (cell_str(r, "n"), "CATALOG".to_string()))
            .collect();
            v.extend(
                run(ds, "SELECT name AS n FROM sys.schemas ORDER BY name", vec![])?
                    .rows()
                    .iter()
                    .map(|r| (cell_str(r, "n"), "SCHEMA".to_string())),
            );
            v
        }
    };
    let columns = vec!["database_name".to_string(), "type".to_string()];
    let mut rows = Vec::new();
    for (name, kind) in raw {
        if let Some(p) = pattern {
            if p != "%" && !like_match(&name, p) {
                continue;
            }
        }
        let mut row = ValueMap::default();
        row.insert("database_name".to_string(), s(&name));
        row.insert("type".to_string(), s(&kind));
        rows.push(row);
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

// -----------------------------------------------
// type="procedures" / "procedure_columns"
// -----------------------------------------------

fn type_procedures(ds: &str, driver: &DbDriver, pattern: Option<&str>) -> CfmlResult {
    let columns = vec![
        "PROCEDURE_CAT".to_string(),
        "PROCEDURE_SCHEM".to_string(),
        "PROCEDURE_NAME".to_string(),
        "REMARKS".to_string(),
        "PROCEDURE_TYPE".to_string(),
    ];
    // (catalog, schema, name, type)
    let raw: Vec<(String, String, String, String)> = match driver {
        // SQLite has no stored procedures.
        DbDriver::Sqlite(_) => Vec::new(),
        DbDriver::Mysql(_) => run(
            ds,
            "SELECT ROUTINE_SCHEMA, ROUTINE_NAME, ROUTINE_TYPE FROM information_schema.ROUTINES \
             WHERE ROUTINE_SCHEMA = DATABASE() ORDER BY ROUTINE_NAME",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "ROUTINE_SCHEMA"),
                String::new(),
                cell_str(r, "ROUTINE_NAME"),
                cell_str(r, "ROUTINE_TYPE"),
            )
        })
        .collect(),
        DbDriver::Postgres(_) => run(
            ds,
            "SELECT routine_catalog, routine_schema, routine_name, routine_type \
             FROM information_schema.routines \
             WHERE routine_schema NOT IN ('pg_catalog','information_schema') \
             ORDER BY routine_name",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "routine_catalog"),
                cell_str(r, "routine_schema"),
                cell_str(r, "routine_name"),
                cell_str(r, "routine_type"),
            )
        })
        .collect(),
        DbDriver::Mssql(_) => run(
            ds,
            "SELECT ROUTINE_CATALOG, ROUTINE_SCHEMA, ROUTINE_NAME, ROUTINE_TYPE \
             FROM INFORMATION_SCHEMA.ROUTINES ORDER BY ROUTINE_NAME",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "ROUTINE_CATALOG"),
                cell_str(r, "ROUTINE_SCHEMA"),
                cell_str(r, "ROUTINE_NAME"),
                cell_str(r, "ROUTINE_TYPE"),
            )
        })
        .collect(),
    };
    let mut rows = Vec::new();
    for (cat, schem, name, kind) in raw {
        if let Some(p) = pattern {
            if !like_match(&name, p) {
                continue;
            }
        }
        let mut row = ValueMap::default();
        row.insert("PROCEDURE_CAT".to_string(), s(&cat));
        row.insert("PROCEDURE_SCHEM".to_string(), s(&schem));
        row.insert("PROCEDURE_NAME".to_string(), s(&name));
        row.insert("REMARKS".to_string(), s(""));
        row.insert("PROCEDURE_TYPE".to_string(), s(&kind));
        rows.push(row);
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

fn type_procedure_columns(ds: &str, driver: &DbDriver, procedure: &str) -> CfmlResult {
    let procedure = fold_case(driver, procedure);
    let columns = vec![
        "PROCEDURE_CAT".to_string(),
        "PROCEDURE_SCHEM".to_string(),
        "PROCEDURE_NAME".to_string(),
        "COLUMN_NAME".to_string(),
        "COLUMN_TYPE".to_string(),
        "TYPE_NAME".to_string(),
        "ORDINAL_POSITION".to_string(),
    ];
    // (proc, column, mode, type, ordinal)
    let raw: Vec<(String, String, String, String, i64)> = match driver {
        DbDriver::Sqlite(_) => Vec::new(),
        DbDriver::Mysql(_) => run(
            ds,
            "SELECT SPECIFIC_NAME, PARAMETER_NAME, PARAMETER_MODE, DATA_TYPE, ORDINAL_POSITION \
             FROM information_schema.PARAMETERS \
             WHERE SPECIFIC_SCHEMA = DATABASE() AND SPECIFIC_NAME = ? \
             ORDER BY ORDINAL_POSITION",
            vec![s(&procedure)],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "SPECIFIC_NAME"),
                cell_str(r, "PARAMETER_NAME"),
                cell_str(r, "PARAMETER_MODE"),
                cell_str(r, "DATA_TYPE"),
                cell_int(r, "ORDINAL_POSITION"),
            )
        })
        .collect(),
        DbDriver::Postgres(_) | DbDriver::Mssql(_) => run(
            ds,
            "SELECT specific_name, parameter_name, parameter_mode, data_type, ordinal_position \
             FROM information_schema.parameters WHERE specific_name LIKE ? \
             ORDER BY ordinal_position",
            // PG/MSSQL mangle specific_name with an oid/number suffix.
            vec![s(&format!("{}%", procedure))],
        )?
        .rows()
        .iter()
        .map(|r| {
            (
                cell_str(r, "specific_name"),
                cell_str(r, "parameter_name"),
                cell_str(r, "parameter_mode"),
                cell_str(r, "data_type"),
                cell_int(r, "ordinal_position"),
            )
        })
        .collect(),
    };
    let mut rows = Vec::new();
    for (proc_name, col, mode, type_name, ordinal) in raw {
        let mut row = ValueMap::default();
        row.insert("PROCEDURE_CAT".to_string(), CfmlValue::Null);
        row.insert("PROCEDURE_SCHEM".to_string(), CfmlValue::Null);
        row.insert("PROCEDURE_NAME".to_string(), s(&proc_name));
        row.insert("COLUMN_NAME".to_string(), s(&col));
        row.insert("COLUMN_TYPE".to_string(), s(&mode));
        row.insert("TYPE_NAME".to_string(), s(&type_name));
        row.insert("ORDINAL_POSITION".to_string(), CfmlValue::Int(ordinal));
        rows.push(row);
    }
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

// -----------------------------------------------
// type="users" / "terms"
// -----------------------------------------------

fn type_users(ds: &str, driver: &DbDriver) -> CfmlResult {
    // Lucee's typeUsers is getSchemas() with TABLE_SCHEM renamed to USER.
    let names: Vec<String> = match driver {
        DbDriver::Sqlite(_) => run(ds, "SELECT name FROM pragma_database_list", vec![])?
            .rows()
            .iter()
            .map(|r| cell_str(r, "name"))
            .collect(),
        DbDriver::Mysql(_) => run(
            ds,
            "SELECT SCHEMA_NAME AS n FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| cell_str(r, "n"))
        .collect(),
        DbDriver::Postgres(_) => run(
            ds,
            "SELECT nspname AS n FROM pg_namespace WHERE nspname NOT LIKE 'pg_%' ORDER BY nspname",
            vec![],
        )?
        .rows()
        .iter()
        .map(|r| cell_str(r, "n"))
        .collect(),
        DbDriver::Mssql(_) => run(ds, "SELECT name AS n FROM sys.schemas ORDER BY name", vec![])?
            .rows()
            .iter()
            .map(|r| cell_str(r, "n"))
            .collect(),
    };
    let columns = vec!["USER".to_string()];
    let rows = names
        .into_iter()
        .map(|n| {
            let mut row = ValueMap::default();
            row.insert("USER".to_string(), s(&n));
            row
        })
        .collect();
    Ok(CfmlValue::Query(CfmlQuery::from_parts(columns, rows)))
}

fn type_terms(driver: &DbDriver) -> CfmlValue {
    let (proc_term, cat_term, schema_term) = match driver {
        DbDriver::Sqlite(_) => ("procedure", "catalog", "schema"),
        DbDriver::Mysql(_) => ("procedure", "database", ""),
        DbDriver::Postgres(_) => ("function", "database", "schema"),
        DbDriver::Mssql(_) => ("procedure", "database", "schema"),
    };
    let mut m = ValueMap::default();
    m.insert("procedure".to_string(), s(proc_term));
    m.insert("catalog".to_string(), s(cat_term));
    m.insert("schema".to_string(), s(schema_term));
    CfmlValue::strukt(m)
}
