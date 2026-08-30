//! Live-MySQL regression tests for `LOAD DATA [LOCAL] INFILE` (GitHub #382).
//! Gated on `RUSTCFML_MYSQL_TEST_URL`, so it is a no-op in a normal `cargo test`
//! run and only exercises a real server when that env var points at one, e.g.
//!
//! ```bash
//! RUSTCFML_MYSQL_TEST_URL='mysql://root:freeze@127.0.0.1:3306/preside_test' \
//!   cargo test -p cfml-stdlib --features all-databases --test mysql_load_data_infile
//! ```
//!
//! The server must be started with `local_infile=ON` for the LOCAL test; a
//! server with it off answers error 1148, which the test reports as such rather
//! than as a routing failure.
//!
//! The routing itself (which statement takes which protocol, and how the path
//! literal decodes) is covered without a server by the `mysql_load_data_tests`
//! unit tests in `builtins.rs`.

#![cfg(feature = "mysql_db")]

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlError;
use std::io::Write;
use std::path::PathBuf;

/// A temp CSV that removes itself. No `tempfile` dev-dependency: this crate has
/// none, and one file in one test does not warrant adding to the dependency
/// (and THIRD-PARTY.txt) surface.
struct TempCsv(PathBuf);

impl Drop for TempCsv {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn exec(url: &str, sql: &str) -> Result<CfmlValue, CfmlError> {
    let mut options = ValueMap::default();
    options.insert("datasource".to_string(), CfmlValue::string(url.to_string()));
    cfml_stdlib::fn_query_execute(vec![
        CfmlValue::string(sql.to_string()),
        CfmlValue::Null,
        CfmlValue::strukt(options),
    ])
}

fn count(url: &str, table: &str) -> i64 {
    let result = exec(url, &format!("select count(*) as n from {}", table))
        .expect("count should succeed");
    let CfmlValue::Query(q) = &result else {
        panic!("expected a query result, got {result:?}");
    };
    let row = q.get_row(0).expect("count should return a row");
    row.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("n"))
        .expect("column n")
        .1
        .as_string()
        .parse()
        .expect("count should be numeric")
}

/// Writes a two-row CSV whose second field on row 1 contains a comma inside
/// quotes, so a green result also proves the server did the CSV parsing (which
/// is the whole point of using LOAD DATA over batched INSERTs).
fn write_csv() -> TempCsv {
    let path = std::env::temp_dir().join(format!(
        "rustcfml_load_data_{}_{}.csv",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut csv = std::fs::File::create(&path).expect("temp csv");
    writeln!(csv, "id,name").unwrap();
    writeln!(csv, "1,\"Smith, John\"").unwrap();
    writeln!(csv, "2,Jane Doe").unwrap();
    csv.flush().unwrap();
    TempCsv(path)
}

#[test]
fn load_data_local_infile_imports_rows() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    let csv = write_csv();
    let path = csv.0.to_str().expect("utf-8 temp path").to_string();

    exec(&url, "drop table if exists _load_data_local").unwrap();
    exec(
        &url,
        "create table _load_data_local ( id int, name text )",
    )
    .unwrap();

    let sql = format!(
        "LOAD DATA LOCAL INFILE '{}' INTO TABLE _load_data_local \
         FIELDS TERMINATED BY ',' ENCLOSED BY '\"' \
         LINES TERMINATED BY '\\n' IGNORE 1 LINES",
        path
    );
    match exec(&url, &sql) {
        Ok(_) => {}
        Err(e) if e.to_string().contains("1148") => {
            eprintln!("skipping: server has local_infile=OFF ({e})");
            return;
        }
        // Before the #382 fix this was error 1295 ("not supported in the
        // prepared statement protocol"), because every mutation was prepared.
        Err(e) => panic!("LOAD DATA LOCAL INFILE should have run: {e}"),
    }

    // The other half of #382: with no client-side file handler registered the
    // mysql crate answers the server's request with an empty buffer, so the
    // statement "succeeds" having imported nothing. Counting the rows is what
    // distinguishes a working import from that silent one.
    assert_eq!(
        count(&url, "_load_data_local"),
        2,
        "both CSV rows should have landed"
    );

    let result = exec(&url, "select name from _load_data_local where id = 1").unwrap();
    let CfmlValue::Query(q) = &result else {
        panic!("expected a query result");
    };
    let row = q.get_row(0).expect("row 1 should exist");
    let name = row
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("name"))
        .expect("column name")
        .1
        .as_string();
    assert_eq!(
        name, "Smith, John",
        "the enclosed comma should have been parsed server-side, not split"
    );

    exec(&url, "drop table if exists _load_data_local").unwrap();
}

#[test]
fn unparseable_local_path_is_refused_not_silently_empty() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    exec(&url, "drop table if exists _load_data_refused").unwrap();
    exec(&url, "create table _load_data_refused ( id int, name text )").unwrap();

    // MySQL does not accept a bind parameter for the filename, and we cannot
    // serve a file we could not name — so this must fail loudly rather than
    // report success having imported nothing.
    let err = exec(
        &url,
        "LOAD DATA LOCAL INFILE :filepath INTO TABLE _load_data_refused",
    )
    .expect_err("an unparseable LOCAL path must be refused");
    assert!(
        err.to_string().contains("LOAD DATA LOCAL INFILE"),
        "the error should name the statement it refused, got: {err}"
    );
    assert_eq!(count(&url, "_load_data_refused"), 0);

    exec(&url, "drop table if exists _load_data_refused").unwrap();
}
