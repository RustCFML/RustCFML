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
        opts.get(k).map(|v| v.as_string())
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
    // A datasource backed by an extension answers for itself. Asked first,
    // because `parse_datasource` below has no variant for one: it falls through
    // its catch-all and classifies the name as a SQLite FILE, which made
    // cfdbinfo emit SQLite pragma SQL at a foreign database. A driver may
    // decline an individual `type=` (returning `None`), and then the engine's
    // own handling runs — so answering `version`/`tables`/`columns` and
    // ignoring the rest is a supported place to start.
    if let Some(name) = attr("datasource") {
        if let Some(driver) = crate::db_driver::lookup_dynamic_datasource(&name) {
            if let Some(answer) = driver.dbinfo(&info_type, args.first().unwrap_or(&CfmlValue::Null))
            {
                return answer;
            }
            return Err(CfmlError::database(format!(
                "cfdbinfo: datasource [{}] is provided by an extension whose driver does not \
                 answer type [{}]",
                name, info_type
            )));
        }
    }

    let driver = parse_datasource(&ds);
    if let DbDriver::Unsupported(sub) = &driver {
        return Err(crate::builtins::unsupported_jdbc_driver_error(sub));
    }

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
    // Case-insensitive: `ValueMap` keys compare without case.
    row.get(name)
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

/// `cell_int`, but distinguishing "absent or not a number" from a real 0.
///
/// Our MySQL driver surfaces a SQL NULL in an information_schema numeric column
/// as an EMPTY STRING, so `cell_int`'s `unwrap_or(0)` silently turned every
/// missing NUMERIC_PRECISION / NUMERIC_SCALE into a legitimate-looking zero —
/// which is why COLUMN_SIZE was 0 for every numeric and temporal column.
fn cell_opt_int(row: &ValueMap, name: &str) -> Option<i64> {
    match cell(row, name) {
        Some(CfmlValue::Int(i)) => Some(*i),
        Some(CfmlValue::Double(d)) => Some(*d as i64),
        Some(CfmlValue::Bool(b)) => Some(*b as i64),
        Some(CfmlValue::Null) | None => None,
        Some(v) => v.as_string().trim().parse::<i64>().ok(),
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
        DbDriver::Unsupported(_) => return false,
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
    // The rest of JDBC's DatabaseMetaData.getColumns set, in its order. Lucee
    // passes the driver's result through, so every driver has these, and
    // `columns_minimal` is exactly these 24 (Lucee 7.1.2, GH #514).
    "SCOPE_CATALOG",
    "SCOPE_SCHEMA",
    "SCOPE_TABLE",
    "SOURCE_DATA_TYPE",
    "IS_AUTOINCREMENT",
    "IS_GENERATEDCOLUMN",
];

const COLUMNS_ENRICHMENT: &[&str] = &[
    "IS_PRIMARYKEY",
    "IS_FOREIGNKEY",
    "REFERENCED_PRIMARYKEY",
    "REFERENCED_PRIMARYKEY_TABLE",
];

/// One source column, driver-neutral, before shaping into the Lucee result.
/// Whether a MySQL `COLUMN_TYPE` is the one Connector/J presents as JDBC BIT.
///
/// Its `tinyInt1isBit` connection property defaults to TRUE, so a SIGNED
/// `TINYINT(1)` — which is also how MySQL and MariaDB store `BOOLEAN` — reads
/// back as BIT. The boundary was measured on Lucee 7.1.0.204, not assumed:
/// `TINYINT(1) UNSIGNED` stays TINYINT, and so does `TINYINT(4)`.
fn mysql_tinyint1_is_bit(column_type: &str) -> bool {
    let lower = column_type.to_lowercase();
    lower.replace(' ', "").starts_with("tinyint(1)") && !lower.contains("unsigned")
}

/// The string length a MySQL temporal column reports as COLUMN_SIZE.
///
/// These are not in `CHARACTER_MAXIMUM_LENGTH` (not character types) nor in
/// `NUMERIC_PRECISION` (not numeric), but JDBC reports the width of the
/// column's textual form. Measured against Lucee 7.1.0.204 on MariaDB 12.1.
fn mysql_temporal_display_size(type_name: &str) -> Option<i64> {
    Some(match type_name.to_ascii_uppercase().as_str() {
        "DATE" => 10,
        "TIME" => 8,
        "YEAR" => 4,
        "DATETIME" | "TIMESTAMP" => 19,
        _ => return None,
    })
}

/// A MySQL spatial type, which Connector/J presents as GEOMETRY (BINARY).
fn mysql_is_spatial(type_name: &str) -> bool {
    matches!(
        type_name.to_ascii_uppercase().as_str(),
        "GEOMETRY" | "POINT" | "LINESTRING" | "POLYGON" | "MULTIPOINT" | "MULTILINESTRING"
            | "MULTIPOLYGON" | "GEOMETRYCOLLECTION" | "GEOMCOLLECTION"
    )
}

/// Connector/J's COLUMN_SIZE for a JSON column (its `MysqlType.JSON` precision).
const MYSQL_JSON_COLUMN_SIZE: i64 = 1_073_741_824;

/// The `java.sql.Types` constant for a SQL type name — `cfdbinfo`'s DATA_TYPE.
///
/// We reported 0 for every column on every driver. Lucee reports the JDBC code,
/// which is what portable schema code switches on. The MySQL/MariaDB rows were
/// measured against Lucee 7.1.0.204 + Connector/J; the rest are the ANSI names'
/// standard `java.sql.Types` values, which are fixed by the JDBC specification
/// and identical across drivers.
fn jdbc_type_code(type_name: &str) -> i64 {
    // An UNSIGNED variant carries the same JDBC code as its signed form
    // (`INT UNSIGNED` is 4, exactly like `INT`), so match on the base word.
    let upper = type_name.to_ascii_uppercase();
    let base = upper.split_whitespace().next().unwrap_or("");
    match base {
        "BIT" => -7,
        "BOOL" | "BOOLEAN" => 16,
        "TINYINT" => -6,
        "SMALLINT" => 5,
        // MySQL's MEDIUMINT has no JDBC code of its own; the driver reports it
        // as INTEGER.
        "INT" | "INTEGER" | "MEDIUMINT" => 4,
        "BIGINT" => -5,
        "DECIMAL" | "DEC" | "FIXED" => 3,
        "NUMERIC" => 2,
        "FLOAT" => 7,
        "REAL" => 7,
        "DOUBLE" => 8,
        // ENUM and SET are presented as CHAR by Connector/J.
        "CHAR" | "ENUM" | "SET" | "NCHAR" | "BPCHAR" => 1,
        // Connector/J presents TINYTEXT (255 bytes) as VARCHAR, the larger
        // TEXT types as LONGVARCHAR.
        "VARCHAR" | "NVARCHAR" | "VARCHAR2" | "CHARACTER" | "TINYTEXT" => 12,
        "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "LONGVARCHAR" | "NTEXT" => -1,
        // `yearIsDateType` defaults to true, so YEAR is DATE.
        "DATE" | "YEAR" => 91,
        "TIME" => 92,
        "DATETIME" | "TIMESTAMP" | "DATETIME2" | "SMALLDATETIME" => 93,
        // Connector/J presents MySQL's spatial types as BINARY.
        "BINARY" | "GEOMETRY" | "POINT" | "LINESTRING" | "POLYGON" | "MULTIPOINT"
        | "MULTILINESTRING" | "MULTIPOLYGON" | "GEOMETRYCOLLECTION" | "GEOMCOLLECTION" => -2,
        // ... and TINYBLOB (255 bytes) as VARBINARY.
        "VARBINARY" | "TINYBLOB" => -3,
        "BLOB" | "MEDIUMBLOB" | "LONGBLOB" | "LONGVARBINARY" | "IMAGE" | "BYTEA" => -4,
        "CLOB" => 2005,
        "UUID" | "UNIQUEIDENTIFIER" => 1111,
        "JSON" | "JSONB" | "XML" => -1,
        // 0 is java.sql.Types.NULL — the value we used to report for
        // everything. It stays the answer only for a type we do not know.
        _ => 0,
    }
}

/// pgjdbc's `TypeInfoCache.getSQLType` for a built-in PostgreSQL type name —
/// the DATA_TYPE Lucee reports. Measured on Lucee 7.1.2.191 + PostgreSQL 18
/// (GH #514): text is VARCHAR, bytea BINARY and bool BIT there, unlike the
/// ANSI names in `jdbc_type_code`.
fn pg_jdbc_type(pg_type: &str) -> i64 {
    match pg_type {
        "int2" => 5,
        "int4" => 4,
        "oid" | "int8" => -5,
        "money" | "float8" => 8,
        "numeric" => 2,
        "float4" => 7,
        "char" | "bpchar" => 1,
        "varchar" | "text" | "name" => 12,
        "bytea" => -2,
        "bool" | "bit" => -7,
        "date" => 91,
        "time" | "timetz" => 92,
        "timestamp" | "timestamptz" => 93,
        "refcursor" => 2012,
        "xml" => 2009,
        _ => 1111, // OTHER: uuid, json, jsonb, inet, …
    }
}

/// pgjdbc's "unknown length", reported for unbounded types.
const PG_UNKNOWN_LENGTH: i64 = i32::MAX as i64;

/// Fractional-seconds digits of a temporal typmod (6 when unconstrained).
fn pg_seconds_digits(typmod: i64) -> i64 {
    if typmod == -1 { 6 } else { typmod }
}

/// pgjdbc's `TypeInfoCache.getDisplaySize`.
fn pg_display_size(pg_type: &str, typmod: i64) -> i64 {
    let frac = |typmod: i64| {
        let d = pg_seconds_digits(typmod);
        if d == 0 { 0 } else { d + 1 }
    };
    match pg_type {
        "int2" => 6,
        "int4" => 11,
        "oid" => 10,
        "int8" => 20,
        "float4" => 15,
        "float8" => 25,
        "char" | "bool" => 1,
        "date" => 13,
        "time" => 8 + frac(typmod),
        "timetz" => 8 + frac(typmod) + 6,
        "timestamp" => 22 + frac(typmod),
        "timestamptz" => 22 + frac(typmod) + 6,
        "interval" => 49,
        "varchar" | "bpchar" if typmod != -1 => typmod - 4,
        "numeric" if typmod != -1 => {
            let precision = ((typmod - 4) >> 16) & 0xFFFF;
            let scale = (typmod - 4) & 0xFFFF;
            1 + precision + if scale != 0 { 1 } else { 0 }
        }
        "numeric" => 131089,
        "bit" => typmod,
        "varbit" if typmod != -1 => typmod,
        _ => PG_UNKNOWN_LENGTH,
    }
}

/// pgjdbc's `TypeInfoCache.getPrecision` — COLUMN_SIZE.
fn pg_precision(pg_type: &str, typmod: i64) -> i64 {
    match pg_type {
        "int2" => 5,
        "oid" | "int4" => 10,
        "int8" => 19,
        "float4" => 8,
        "float8" => 17,
        "numeric" if typmod == -1 => 0,
        "numeric" => ((typmod - 4) >> 16) & 0xFFFF,
        "char" | "bool" => 1,
        "varchar" | "bpchar" if typmod == -1 => PG_UNKNOWN_LENGTH,
        "varchar" | "bpchar" => typmod - 4,
        "date" | "time" | "timetz" | "interval" | "timestamp" | "timestamptz" => {
            pg_display_size(pg_type, typmod)
        }
        "bit" => typmod,
        "varbit" if typmod == -1 => PG_UNKNOWN_LENGTH,
        "varbit" => typmod,
        _ => PG_UNKNOWN_LENGTH,
    }
}

/// pgjdbc's `TypeInfoCache.getScale` — DECIMAL_DIGITS (0 when not applicable).
fn pg_scale(pg_type: &str, typmod: i64) -> i64 {
    match pg_type {
        "float4" => 8,
        "float8" => 17,
        "numeric" if typmod == -1 => 0,
        "numeric" => (typmod - 4) & 0xFFFF,
        "time" | "timetz" | "timestamp" | "timestamptz" => pg_seconds_digits(typmod),
        "interval" if typmod == -1 => 6,
        "interval" => typmod & 0xFFFF,
        _ => 0,
    }
}

/// The columns mssql-jdbc's `getColumns` passes through from `sp_columns_100`
/// after the 24 JDBC ones, in its order. Lucee reports them before its own
/// IS_PRIMARYKEY enrichment (measured on 7.1.2.191, GH #539).
const MSSQL_EXTRA_COLUMNS: &[&str] = &[
    "SS_IS_SPARSE",
    "SS_IS_COLUMN_SET",
    "SS_UDT_CATALOG_NAME",
    "SS_UDT_SCHEMA_NAME",
    "SS_UDT_ASSEMBLY_TYPE_NAME",
    "SS_XML_SCHEMACOLLECTION_CATALOG_NAME",
    "SS_XML_SCHEMACOLLECTION_SCHEMA_NAME",
    "SS_XML_SCHEMACOLLECTION_NAME",
];

/// The driver's own `getColumns` columns that follow IS_GENERATEDCOLUMN.
fn driver_extra_columns(driver: &DbDriver) -> &'static [&'static str] {
    match driver {
        DbDriver::Mssql(_) => MSSQL_EXTRA_COLUMNS,
        _ => &[],
    }
}

/// mssql-jdbc's `DataTypeFilter`: `sp_columns_100` answers with ODBC type
/// codes, and the driver rewrites the ones that differ in `java.sql.Types`.
fn mssql_jdbc_type(odbc: i64) -> i64 {
    match odbc {
        6 => 8,         // SQL_FLOAT → DOUBLE
        -8 => -15,      // SQL_WCHAR → NCHAR
        -10 => -16,     // SQL_WLONGVARCHAR → LONGNVARCHAR
        -11 => 1,       // SQL_GUID → CHAR
        -152 => -16,    // SQL_SS_XML → LONGNVARCHAR
        -154 => 92,     // SQL_SS_TIME2 → TIME
        -151 => -3,     // SQL_SS_UDT (geography, hierarchyid) → VARBINARY
        other => other, // datetimeoffset stays -155 (microsoft.sql.Types)
    }
}

/// mssql-jdbc's `ZeroFixupFilter`: a `(max)` column has length 0 in the
/// catalogue, and the driver reports Integer.MAX_VALUE instead.
fn mssql_zero_fixup(v: Option<i64>) -> Option<i64> {
    v.map(|n| if n == 0 { i32::MAX as i64 } else { n })
}

struct ColInfo {
    table_cat: String,
    table_schem: String,
    table_name: String,
    column_name: String,
    type_name: String,
    column_size: i64,
    /// `Option`, not a bare number: JDBC reports DECIMAL_DIGITS as SQL NULL for
    /// a non-numeric column, and Lucee passes that through as an empty cell. We
    /// reported 0, which reads as "zero decimal places" — a real scale — on
    /// every VARCHAR and DATE column.
    decimal_digits: Option<i64>,
    nullable: bool,
    default_value: Option<String>,
    ordinal: i64,
    is_pk: bool,
    is_autoincrement: bool,
    jdbc: JdbcColumnExtras,
    /// The driver's own columns after IS_GENERATEDCOLUMN, in
    /// [`driver_extra_columns`] order (mssql-jdbc's `SS_*`).
    driver_extra: Vec<CfmlValue>,
}

/// The `getColumns` fields whose values are driver-specific. `None` is SQL
/// NULL, an empty cell in the result.
struct JdbcColumnExtras {
    /// The `java.sql.Types` code; `None` derives it from TYPE_NAME.
    data_type: Option<i64>,
    buffer_length: Option<i64>,
    num_prec_radix: Option<i64>,
    remarks: String,
    sql_data_type: Option<i64>,
    sql_datetime_sub: Option<i64>,
    char_octet_length: Option<i64>,
    source_data_type: Option<i64>,
    is_generated: bool,
}

impl JdbcColumnExtras {
    /// What every driver reported before these were driver-specific: zeros,
    /// and CHAR_OCTET_LENGTH equal to COLUMN_SIZE. Kept for the drivers not
    /// yet measured against Lucee field by field.
    fn legacy(column_size: i64) -> Self {
        JdbcColumnExtras {
            data_type: None,
            buffer_length: Some(0),
            num_prec_radix: Some(10),
            remarks: String::new(),
            sql_data_type: Some(0),
            sql_datetime_sub: Some(0),
            char_octet_length: Some(column_size),
            source_data_type: None,
            is_generated: false,
        }
    }
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
                    decimal_digits: None,
                    nullable: cell_int(&row, "notnull") == 0,
                    default_value: dflt,
                    ordinal: cell_int(&row, "cid") + 1,
                    is_pk: cell_int(&row, "pk") > 0,
                    is_autoincrement,
                    jdbc: JdbcColumnExtras::legacy(size),
                    driver_extra: Vec::new(),
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
                     CHARACTER_MAXIMUM_LENGTH, CHARACTER_OCTET_LENGTH, NUMERIC_PRECISION, \
                     NUMERIC_SCALE, DATETIME_PRECISION, IS_NULLABLE, \
                     COLUMN_DEFAULT, ORDINAL_POSITION, COLUMN_KEY, EXTRA, COLUMN_COMMENT \
                     FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    vec![s(sc), s(&table)],
                ),
                None => (
                    "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, \
                     CHARACTER_MAXIMUM_LENGTH, CHARACTER_OCTET_LENGTH, NUMERIC_PRECISION, \
                     NUMERIC_SCALE, DATETIME_PRECISION, IS_NULLABLE, \
                     COLUMN_DEFAULT, ORDINAL_POSITION, COLUMN_KEY, EXTRA, COLUMN_COMMENT \
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
                // Connector/J's `tinyInt1isBit=true` (its DEFAULT) presents a
                // SIGNED `TINYINT(1)` — which is also how MySQL/MariaDB store
                // `BOOLEAN` — as JDBC BIT, and Lucee inherits that. We reported
                // "tinyint", so Preside's schema sync saw a type change on
                // every boolean column and tried to alter it on every dbSync
                // (GH #390 create path, #391 alter path).
                //
                // Measured boundary, NOT assumed: `TINYINT(1) UNSIGNED` stays
                // TINYINT on Lucee — the driver quirk applies to the signed
                // form only — and `TINYINT(4)` stays TINYINT.
                let is_bool_tinyint = mysql_tinyint1_is_bit(&column_type);
                let type_name = if is_bool_tinyint {
                    "BIT".to_string()
                } else if mysql_is_spatial(&type_name) {
                    // Every spatial type is GEOMETRY to Connector/J (GH #539).
                    "GEOMETRY".to_string()
                } else {
                    type_name.to_uppercase()
                };
                // CHARACTER_MAXIMUM_LENGTH comes back as an EMPTY STRING (not a
                // typed NULL) for a non-character column, so this fallback never
                // ran and every numeric/temporal column reported COLUMN_SIZE 0.
                let char_len = match cell(&row, "CHARACTER_MAXIMUM_LENGTH") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => v.as_string().trim().parse::<i64>().ok(),
                };
                let size = if is_bool_tinyint {
                    1
                } else if type_name == "JSON" {
                    // Connector/J's fixed precision for JSON (MysqlType.JSON);
                    // information_schema has no length for it.
                    MYSQL_JSON_COLUMN_SIZE
                } else if mysql_is_spatial(&type_name) {
                    // MysqlType.GEOMETRY's precision.
                    65535
                } else {
                    match char_len {
                        // Connector/J clamps a length to Integer.MAX_VALUE, so
                        // LONGTEXT reports 2147483647, not MySQL's 4294967295.
                        Some(n) => n.min(i32::MAX as i64),
                        None => match mysql_temporal_display_size(&type_name) {
                            // `datetime(3)` is 23 wide: the point and the digits.
                            Some(n) => match cell_opt_int(&row, "DATETIME_PRECISION") {
                                Some(fsp) if fsp > 0 && type_name != "DATE" && type_name != "YEAR" => {
                                    n + fsp + 1
                                }
                                _ => n,
                            },
                            None => cell_int(&row, "NUMERIC_PRECISION"),
                        },
                    }
                };
                let dflt = match cell(&row, "COLUMN_DEFAULT") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                // JDBC's DECIMAL_DIGITS is "not applicable" — SQL NULL — for
                // an EXACT integer type, and 0 for an approximate one, which is
                // the INVERSE of what information_schema stores (NUMERIC_SCALE
                // is 0 for INT and NULL for DOUBLE). Lucee reports the JDBC
                // reading, so map it rather than passing the catalogue value
                // straight through.
                let decimal_digits = match type_name.as_str() {
                    "DECIMAL" | "NUMERIC" | "DEC" | "FIXED" => cell_opt_int(&row, "NUMERIC_SCALE"),
                    // An approximate type is 0 unless declared with a scale:
                    // `float(7,3)` reports 3 (Lucee 7.1.2.191, GH #539).
                    "FLOAT" | "DOUBLE" | "REAL" => Some(cell_opt_int(&row, "NUMERIC_SCALE").unwrap_or(0)),
                    _ => None,
                };
                cols.push(ColInfo {
                    table_cat: cell_str(&row, "TABLE_SCHEMA"),
                    table_schem: String::new(),
                    table_name: cell_str(&row, "TABLE_NAME"),
                    column_name: cell_str(&row, "COLUMN_NAME"),
                    type_name,
                    column_size: size,
                    decimal_digits,
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
                    jdbc: JdbcColumnExtras {
                        // Measured on Lucee 7.1.2.191 + MySQL 8.4: the column
                        // comment is REMARKS, and EXTRA "STORED GENERATED" /
                        // "VIRTUAL GENERATED" is IS_GENERATEDCOLUMN.
                        remarks: cell_str(&row, "COLUMN_COMMENT"),
                        is_generated: cell_str(&row, "EXTRA").to_lowercase().contains("generated"),
                        // Connector/J reports a constant 65535 BUFFER_LENGTH,
                        // and CHAR_OCTET_LENGTH is the catalogue's byte length
                        // (4 × the length for utf8mb4), NULL for a non-character
                        // column, clamped like COLUMN_SIZE (GH #539).
                        buffer_length: Some(65535),
                        char_octet_length: cell_opt_int(&row, "CHARACTER_OCTET_LENGTH")
                            .map(|n| n.min(i32::MAX as i64)),
                        ..JdbcColumnExtras::legacy(size)
                    },
                    driver_extra: Vec::new(),
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
            // pgjdbc's DatabaseMetaData.getColumns, which is what Lucee reports
            // (GH #514): straight from pg_catalog, so the type's OID-level facts
            // (typmod, array element, domain base, identity, generated) are
            // there to derive COLUMN_SIZE / DECIMAL_DIGITS / DATA_TYPE exactly
            // as the driver does. information_schema had none of that, and
            // reported serial columns as plain int4.
            let base = run(
                ds,
                "SELECT current_database() AS table_cat, n.nspname AS table_schem, \
                 c.relname AS table_name, a.attname AS column_name, t.typname AS type_name, \
                 t.typtype::text AS typtype, a.atttypmod AS typmod, \
                 CASE WHEN t.typcategory = 'A' THEN et.typname END AS elem_name, \
                 bt.typname AS base_name, t.typtypmod AS base_typmod, \
                 CASE WHEN a.attnotnull OR (t.typtype = 'd' AND t.typnotnull) THEN 1 ELSE 0 END AS notnull, \
                 row_number() OVER (ORDER BY a.attnum) AS ordinal, \
                 a.attidentity::text AS attidentity, a.attgenerated::text AS attgenerated, \
                 pg_catalog.pg_get_expr(def.adbin, def.adrelid) AS adsrc, dsc.description AS remarks \
                 FROM pg_catalog.pg_namespace n \
                 JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid \
                 JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid \
                 JOIN pg_catalog.pg_type t ON a.atttypid = t.oid \
                 LEFT JOIN pg_catalog.pg_type et ON et.oid = t.typelem \
                 LEFT JOIN pg_catalog.pg_type bt ON bt.oid = t.typbasetype \
                 LEFT JOIN pg_catalog.pg_attrdef def ON a.attrelid = def.adrelid AND a.attnum = def.adnum \
                 LEFT JOIN pg_catalog.pg_description dsc ON c.oid = dsc.objoid AND a.attnum = dsc.objsubid \
                   AND dsc.classoid = 'pg_catalog.pg_class'::regclass \
                 WHERE c.relkind IN ('r', 'p', 'v', 'f', 'm') AND a.attnum > 0 AND NOT a.attisdropped \
                   AND n.nspname = COALESCE(NULLIF(?, ''), current_schema()) AND c.relname = ? \
                 ORDER BY a.attnum",
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
                let pg_name = cell_str(&row, "type_name");
                let typtype = cell_str(&row, "typtype");
                let typmod = cell_int(&row, "typmod");
                let elem = cell_str(&row, "elem_name");
                let dflt = match cell(&row, "adsrc") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                // The type whose rules give the size: an array's element (at
                // the column's typmod), or a domain's base type.
                let base_name = cell_str(&row, "base_name");
                let (rule_name, rule_typmod): (&str, i64) = if !elem.is_empty() {
                    (&elem, typmod)
                } else if typtype == "d" {
                    (&base_name, typmod)
                } else {
                    (&pg_name, typmod)
                };
                let sql_type = if typtype == "c" {
                    2002 // STRUCT
                } else if typtype == "d" {
                    2001 // DISTINCT
                } else if typtype == "e" {
                    12 // VARCHAR
                } else if !elem.is_empty() {
                    2003 // ARRAY
                } else {
                    pg_jdbc_type(&pg_name)
                };
                let size_of = |name: &str, typmod: i64| {
                    let p = pg_precision(name, typmod);
                    if p == 0 && name != "numeric" { pg_display_size(name, typmod) } else { p }
                };
                let base_typmod = cell_int(&row, "base_typmod");
                // pgjdbc reads a CONSTRAINED domain's size straight off its
                // typmod — raw, so `varchar(7)` reports 11 and `timestamp(3)`
                // reports 3 — decoding it only for numeric, and takes the
                // scale from the unconstrained base type. Measured on Lucee
                // 7.1.2.191; odd, but it is what portable code sees there.
                let (column_size, decimal_digits) = if typtype == "d" && base_typmod != -1 {
                    if rule_name == "numeric" {
                        (((base_typmod - 4) >> 16) & 0xFFFF, Some((base_typmod - 4) & 0xFFFF))
                    } else {
                        (base_typmod, Some(pg_scale(rule_name, -1)))
                    }
                } else if typtype == "d" {
                    (size_of(rule_name, -1), Some(pg_scale(rule_name, -1)))
                } else {
                    // An unconstrained numeric has no scale: SQL NULL.
                    let scale = (rule_name != "numeric" || rule_typmod != -1)
                        .then(|| pg_scale(rule_name, rule_typmod));
                    (size_of(rule_name, rule_typmod), scale)
                };
                // pgjdbc reports serial / bigserial / smallserial by name when
                // the default draws from a sequence.
                let is_serial = dflt.as_deref().is_some_and(|d| d.contains("nextval("));
                let type_name = match (is_serial, pg_name.as_str()) {
                    (true, "int4") => "serial".to_string(),
                    (true, "int8") => "bigserial".to_string(),
                    (true, "int2") => "smallserial".to_string(),
                    _ => pg_name.clone(),
                };
                let identity = !cell_str(&row, "attidentity").is_empty();
                let generated = !cell_str(&row, "attgenerated").is_empty();
                let name = cell_str(&row, "column_name");
                cols.push(ColInfo {
                    table_cat: cell_str(&row, "table_cat"),
                    table_schem: cell_str(&row, "table_schem"),
                    table_name: cell_str(&row, "table_name"),
                    is_pk: pk_set.contains(&name.to_lowercase()),
                    column_name: name,
                    type_name,
                    column_size,
                    decimal_digits,
                    nullable: cell_int(&row, "notnull") == 0,
                    is_autoincrement: is_serial || identity,
                    default_value: dflt,
                    ordinal: cell_int(&row, "ordinal"),
                    jdbc: JdbcColumnExtras {
                        data_type: Some(sql_type),
                        buffer_length: None,
                        num_prec_radix: Some(if matches!(rule_name, "bit" | "varbit") { 2 } else { 10 }),
                        remarks: cell_str(&row, "remarks"),
                        sql_data_type: None,
                        sql_datetime_sub: None,
                        char_octet_length: matches!(sql_type, 1 | 12 | -1 | -2 | -3 | -4)
                            .then_some(column_size),
                        source_data_type: (typtype == "d").then(|| pg_jdbc_type(rule_name)),
                        is_generated: generated,
                    },
                    driver_extra: Vec::new(),
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
            // mssql-jdbc's DatabaseMetaData.getColumns, which is what Lucee
            // reports (GH #539): the `sp_columns_100` catalogue procedure, then
            // the driver's fix-ups (see `mssql_jdbc_type` and
            // `mssql_zero_fixup`). INFORMATION_SCHEMA had no identity or
            // computed flag and no length for a `(max)` column, and its NULL
            // NUMERIC_PRECISION gave every date and bit column COLUMN_SIZE 0.
            // `@fUsePattern = 0`: the table name is matched exactly, as before,
            // so an underscore in it is not a wildcard.
            let base = run(
                ds,
                "EXEC sp_columns_100 @table_name = ?, @table_owner = ?, @ODBCVer = 3, @fUsePattern = 0",
                vec![s(&table), schema.map_or(CfmlValue::Null, s)],
            )?;
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
                let dflt = match cell(&row, "COLUMN_DEF") {
                    Some(CfmlValue::Null) | None => None,
                    Some(v) => Some(v.as_string()),
                };
                let name = cell_str(&row, "COLUMN_NAME");
                let column_size = mssql_zero_fixup(cell_opt_int(&row, "PRECISION")).unwrap_or(0);
                cols.push(ColInfo {
                    table_cat: cell_str(&row, "TABLE_QUALIFIER"),
                    table_schem: cell_str(&row, "TABLE_OWNER"),
                    table_name: cell_str(&row, "TABLE_NAME"),
                    is_pk: pk_set.contains(&name.to_lowercase()),
                    column_name: name,
                    // "int identity" for an identity column, as mssql-jdbc reports.
                    type_name: cell_str(&row, "TYPE_NAME"),
                    column_size,
                    decimal_digits: cell_opt_int(&row, "SCALE"),
                    nullable: cell_int(&row, "NULLABLE") != 0,
                    default_value: dflt,
                    ordinal: cell_int(&row, "ORDINAL_POSITION"),
                    is_autoincrement: cell_int(&row, "SS_IS_IDENTITY") != 0,
                    jdbc: JdbcColumnExtras {
                        data_type: Some(mssql_jdbc_type(cell_int(&row, "DATA_TYPE"))),
                        buffer_length: mssql_zero_fixup(cell_opt_int(&row, "LENGTH")),
                        num_prec_radix: cell_opt_int(&row, "RADIX"),
                        remarks: cell_str(&row, "REMARKS"),
                        sql_data_type: cell_opt_int(&row, "SQL_DATA_TYPE"),
                        sql_datetime_sub: cell_opt_int(&row, "SQL_DATETIME_SUB"),
                        char_octet_length: mssql_zero_fixup(cell_opt_int(&row, "CHAR_OCTET_LENGTH")),
                        source_data_type: cell_opt_int(&row, "SS_DATA_TYPE"),
                        is_generated: cell_int(&row, "SS_IS_COMPUTED") != 0,
                    },
                    driver_extra: MSSQL_EXTRA_COLUMNS
                        .iter()
                        .map(|c| cell(&row, c).cloned().unwrap_or(CfmlValue::Null))
                        .collect(),
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

    let extra_columns = driver_extra_columns(driver);
    let mut out_columns: Vec<String> = COLUMNS_COLUMNS.iter().map(|c| c.to_string()).collect();
    out_columns.extend(extra_columns.iter().map(|c| c.to_string()));
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
        let int_or_null = |v: Option<i64>| match v {
            Some(n) => CfmlValue::Int(n),
            None => CfmlValue::Null,
        };
        row.insert(
            "DATA_TYPE".to_string(),
            CfmlValue::Int(c.jdbc.data_type.unwrap_or_else(|| jdbc_type_code(&c.type_name))),
        );
        row.insert("TYPE_NAME".to_string(), s(&c.type_name));
        row.insert("COLUMN_SIZE".to_string(), CfmlValue::Int(c.column_size));
        row.insert("BUFFER_LENGTH".to_string(), int_or_null(c.jdbc.buffer_length));
        row.insert(
            "DECIMAL_DIGITS".to_string(),
            match c.decimal_digits {
                Some(d) => CfmlValue::Int(d),
                None => CfmlValue::Null,
            },
        );
        row.insert("NUM_PREC_RADIX".to_string(), int_or_null(c.jdbc.num_prec_radix));
        row.insert(
            "NULLABLE".to_string(),
            CfmlValue::Int(if c.nullable { 1 } else { 0 }),
        );
        row.insert("REMARKS".to_string(), s(&c.jdbc.remarks));
        row.insert(
            "COLUMN_DEFAULT_VALUE".to_string(),
            match &c.default_value {
                Some(d) => s(d),
                None => CfmlValue::Null,
            },
        );
        row.insert("SQL_DATA_TYPE".to_string(), int_or_null(c.jdbc.sql_data_type));
        row.insert("SQL_DATETIME_SUB".to_string(), int_or_null(c.jdbc.sql_datetime_sub));
        row.insert("CHAR_OCTET_LENGTH".to_string(), int_or_null(c.jdbc.char_octet_length));
        row.insert("ORDINAL_POSITION".to_string(), CfmlValue::Int(c.ordinal));
        row.insert(
            "IS_NULLABLE".to_string(),
            s(if c.nullable { "YES" } else { "NO" }),
        );
        row.insert("SCOPE_CATALOG".to_string(), CfmlValue::Null);
        row.insert("SCOPE_SCHEMA".to_string(), CfmlValue::Null);
        row.insert("SCOPE_TABLE".to_string(), CfmlValue::Null);
        row.insert("SOURCE_DATA_TYPE".to_string(), int_or_null(c.jdbc.source_data_type));
        row.insert(
            "IS_AUTOINCREMENT".to_string(),
            s(if c.is_autoincrement { "YES" } else { "NO" }),
        );
        row.insert(
            "IS_GENERATEDCOLUMN".to_string(),
            s(if c.jdbc.is_generated { "YES" } else { "NO" }),
        );
        for (name, value) in extra_columns.iter().zip(c.driver_extra) {
            row.insert(name.to_string(), value);
        }
        if enrich {
            row.insert(
                "IS_PRIMARYKEY".to_string(),
                s(if c.is_pk { "YES" } else { "NO" }),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Unsupported(sub) => return Err(crate::builtins::unsupported_jdbc_driver_error(sub)),
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
        DbDriver::Sqlite(_) | DbDriver::Unsupported(_) => ("procedure", "catalog", "schema"),
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

#[cfg(test)]
mod type_mapping_tests {
    use super::*;

    #[test]
    fn tinyint1_is_bit_only_when_signed() {
        // The rows measured against Lucee 7.1.0.204 + Connector/J on MariaDB.
        assert!(mysql_tinyint1_is_bit("tinyint(1)"));
        assert!(mysql_tinyint1_is_bit("TINYINT(1)"));
        // `boolean` is stored as tinyint(1), so it arrives here spelled that way.
        assert!(mysql_tinyint1_is_bit("tinyint( 1 )"));
        // The boundary: unsigned keeps its own type, and so does any other width.
        assert!(!mysql_tinyint1_is_bit("tinyint(1) unsigned"));
        assert!(!mysql_tinyint1_is_bit("tinyint(4)"));
        assert!(!mysql_tinyint1_is_bit("tinyint"));
        assert!(!mysql_tinyint1_is_bit("bit(1)"));
        assert!(!mysql_tinyint1_is_bit("int(11)"));
    }

    #[test]
    fn jdbc_codes_match_lucee() {
        // Every pair here was read off Lucee 7.1.0.204 for a real column.
        for (name, code) in [
            ("BIT", -7),
            ("TINYINT", -6),
            ("SMALLINT", 5),
            ("MEDIUMINT", 4),
            ("INT", 4),
            ("BIGINT", -5),
            ("DECIMAL", 3),
            ("FLOAT", 7),
            ("DOUBLE", 8),
            ("CHAR", 1),
            ("ENUM", 1),
            ("VARCHAR", 12),
            ("TEXT", -1),
            ("LONGTEXT", -1),
            ("DATE", 91),
            ("TIME", 92),
            ("DATETIME", 93),
            ("TIMESTAMP", 93),
            ("BINARY", -2),
            ("VARBINARY", -3),
            ("BLOB", -4),
            // Lucee 7.1.2.191 + Connector/J on MySQL 8.4 (GH #539).
            ("YEAR", 91),
            ("TINYTEXT", 12),
            ("MEDIUMTEXT", -1),
            ("TINYBLOB", -3),
            ("GEOMETRY", -2),
            ("POINT", -2),
        ] {
            assert_eq!(jdbc_type_code(name), code, "{name}");
        }
    }

    #[test]
    fn an_unsigned_variant_keeps_its_base_code() {
        assert_eq!(jdbc_type_code("INT UNSIGNED"), jdbc_type_code("INT"));
        assert_eq!(jdbc_type_code("TINYINT UNSIGNED"), jdbc_type_code("TINYINT"));
        assert_eq!(jdbc_type_code("BIGINT UNSIGNED"), jdbc_type_code("BIGINT"));
    }

    #[test]
    fn an_unknown_type_still_reports_types_null() {
        // 0 is java.sql.Types.NULL — what we used to report for EVERY column.
        assert_eq!(jdbc_type_code("SOME_VENDOR_TYPE"), 0);
    }

    #[test]
    fn temporal_display_sizes_match_lucee() {
        assert_eq!(mysql_temporal_display_size("DATE"), Some(10));
        assert_eq!(mysql_temporal_display_size("TIME"), Some(8));
        assert_eq!(mysql_temporal_display_size("DATETIME"), Some(19));
        assert_eq!(mysql_temporal_display_size("TIMESTAMP"), Some(19));
        assert_eq!(mysql_temporal_display_size("YEAR"), Some(4));
        // A non-temporal type falls through to NUMERIC_PRECISION instead.
        assert_eq!(mysql_temporal_display_size("INT"), None);
    }

    /// pgjdbc's TypeInfoCache, as Lucee 7.1.2.191 reported it on PostgreSQL 18
    /// (GH #514). `typmod` is pg_attribute.atttypmod: -1 when unconstrained,
    /// length + 4 for varchar/bpchar, ((precision << 16) | scale) + 4 for
    /// numeric, the fractional digits for temporal types.
    #[test]
    fn pg_sizes_and_scales_match_pgjdbc() {
        for (ty, typmod, size, scale) in [
            ("int4", -1, 10, 0),
            ("int8", -1, 19, 0),
            ("int2", -1, 5, 0),
            ("bool", -1, 1, 0),
            ("varchar", 24, 20, 0),
            ("varchar", -1, 2147483647, 0),
            ("bpchar", 7, 3, 0),
            ("text", -1, 2147483647, 0),
            ("timestamp", -1, 29, 6),
            ("timestamp", 0, 22, 0),
            ("timestamptz", 3, 32, 3),
            ("time", -1, 15, 6),
            ("timetz", 2, 17, 2),
            ("date", -1, 13, 0),
            ("numeric", (10 << 16 | 2) + 4, 10, 2),
            ("numeric", -1, 0, 0),
            ("float4", -1, 8, 8),
            ("float8", -1, 17, 17),
            ("interval", -1, 49, 6),
            ("bit", 3, 3, 0),
            ("uuid", -1, 2147483647, 0),
        ] {
            let mut got = pg_precision(ty, typmod);
            if got == 0 && ty != "numeric" {
                got = pg_display_size(ty, typmod);
            }
            assert_eq!(got, size, "COLUMN_SIZE of {ty}({typmod})");
            assert_eq!(pg_scale(ty, typmod), scale, "DECIMAL_DIGITS of {ty}({typmod})");
        }
    }

    #[test]
    fn pg_type_codes_match_pgjdbc() {
        for (ty, code) in [
            ("int4", 4), ("int8", -5), ("int2", 5), ("bool", -7), ("varchar", 12), ("text", 12),
            ("bpchar", 1), ("bytea", -2), ("numeric", 2), ("float4", 7), ("float8", 8),
            ("date", 91), ("time", 92), ("timestamp", 93), ("timestamptz", 93),
            ("uuid", 1111), ("jsonb", 1111), ("interval", 1111),
        ] {
            assert_eq!(pg_jdbc_type(ty), code, "DATA_TYPE of {ty}");
        }
    }
}
