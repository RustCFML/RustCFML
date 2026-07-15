//! Live-MySQL regression tests for JDBC result-set -> CFML value mapping of
//! the column types called out in GH #273 (DATE/DATETIME) and GH #274 (BIT).
//! Gated on `RUSTCFML_MYSQL_TEST_URL`, so it is a no-op in a normal `cargo test`
//! run and only exercises a real server when that env var points at one, e.g.
//!
//! ```bash
//! RUSTCFML_MYSQL_TEST_URL='mysql://root:freeze@127.0.0.1:3306/preside_test' \
//!   cargo test -p cfml-stdlib --features all-databases --test mysql_column_types
//! ```

#![cfg(feature = "mysql_db")]

use cfml_common::dynamic::{CfmlValue, ValueMap};

fn ds_opts(url: &str) -> ValueMap {
    let mut options = ValueMap::default();
    options.insert("datasource".to_string(), CfmlValue::string(url.to_string()));
    options
}

fn exec(url: &str, sql: &str) -> CfmlValue {
    let args = vec![
        CfmlValue::string(sql.to_string()),
        CfmlValue::Null,
        CfmlValue::strukt(ds_opts(url)),
    ];
    cfml_stdlib::fn_query_execute(args).expect("query should succeed")
}

/// Pull column `col`, row 0, from a query result as a display string.
fn first_cell(result: &CfmlValue, col: &str) -> String {
    let CfmlValue::Query(q) = result else {
        panic!("expected a query result, got {result:?}");
    };
    let row = q.get_row(0).expect("query should have at least one row");
    row.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(col))
        .unwrap_or_else(|| panic!("column {col} not found"))
        .1
        .as_string()
}

#[test]
fn bit_columns_read_back_as_integers() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    exec(&url, "drop table if exists _bit_map");
    exec(
        &url,
        "create table _bit_map ( one bit not null, wide bit(16) not null )",
    );
    exec(&url, "insert into _bit_map values (1, b'0000000100000000')");
    let q = exec(&url, "select one, wide from _bit_map");

    // GH #274: BIT(1) -> 1 (not an empty string / control byte); wider BIT ->
    // the big-endian integer value.
    assert_eq!(first_cell(&q, "one"), "1", "BIT(1) should read back as 1");
    assert_eq!(first_cell(&q, "wide"), "256", "BIT(16) 0x0100 should read back as 256");

    exec(&url, "drop table if exists _bit_map");
}

#[test]
fn date_and_datetime_columns_keep_full_datetime() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    exec(&url, "drop table if exists _dt_map");
    exec(&url, "create table _dt_map ( d datetime, dt date )");
    exec(
        &url,
        "insert into _dt_map values ('1990-01-01 00:00:00', '1990-01-01')",
    );
    let q = exec(&url, "select d, dt from _dt_map");

    // GH #273: a DATETIME must not drop its (midnight) time to a bare date, and
    // a DATE column is surfaced as a datetime at midnight — matching now() /
    // createDateTime's canonical `YYYY-MM-DD HH:MM:SS` form.
    assert_eq!(
        first_cell(&q, "d"),
        "1990-01-01 00:00:00",
        "DATETIME at midnight must keep its time component"
    );
    assert_eq!(
        first_cell(&q, "dt"),
        "1990-01-01 00:00:00",
        "DATE column should be promoted to a datetime at midnight"
    );

    exec(&url, "drop table if exists _dt_map");
}
