//! Live-MySQL regression test: session/user variables set by one statement must
//! survive to the next statement on the same datasource, because pooled
//! connections are no longer reset on return-to-pool and the pool keeps a serial
//! workload on a single connection (`reset_connection = false`, `pool_min = 1`).
//!
//! This is what JDBC — and therefore Lucee/ACF — do, and what mysqldump-style
//! install scripts rely on. Masa CMS's `core/setup/db/mysql.sql` opens with
//! `SET @OLD_SQL_MODE=@@SQL_MODE, SQL_MODE='NO_AUTO_VALUE_ON_ZERO'` and later
//! restores with `SET SQL_MODE=@OLD_SQL_MODE`; before this fix the crate's
//! default 10-connection FIFO pool + connection reset left `@OLD_SQL_MODE` NULL
//! when the restore ran, and MariaDB rejects `SET sql_mode = NULL` (ERROR 1231).
//!
//! Gated on `RUSTCFML_MYSQL_TEST_URL`, so it is a no-op in normal `cargo test`
//! runs and only exercises a real server when that env var points at one, e.g.
//!
//! ```bash
//! RUSTCFML_MYSQL_TEST_URL='mysql://root:freeze@127.0.0.1:3306/masa' \
//!   cargo test -p cfml-stdlib --features all-databases --test mysql_session_var_persistence
//! ```

#![cfg(feature = "mysql_db")]

use cfml_common::dynamic::{CfmlValue, ValueMap};

fn run(url: &str, sql: &str) -> cfml_common::vm::CfmlResult {
    let mut options = ValueMap::default();
    options.insert("datasource".to_string(), CfmlValue::string(url.to_string()));
    let args = vec![
        CfmlValue::string(sql.to_string()),
        CfmlValue::Null,
        CfmlValue::strukt(options),
    ];
    cfml_stdlib::fn_query_execute(args)
}

#[test]
fn user_variable_persists_across_statements() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    run(&url, "SET @rustcfml_probe = 42").expect("SET user var should succeed");
    let result = run(&url, "SELECT @rustcfml_probe AS v")
        .expect("SELECT should succeed on the same (unreset) connection");

    let value = format!("{:?}", result);
    assert!(
        value.contains("42"),
        "user variable set on a prior statement should still be readable, got: {}",
        value
    );
}

#[test]
fn sql_mode_save_restore_round_trip() {
    // The exact Masa mysql.sql preamble/epilogue pattern.
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    run(
        &url,
        "SET @OLD_SQL_MODE=@@SQL_MODE, SQL_MODE='NO_AUTO_VALUE_ON_ZERO'",
    )
    .expect("saving sql_mode should succeed");

    // Before the fix this failed with ERROR 1231 because @OLD_SQL_MODE was NULL
    // on the (different / reset) connection this statement landed on.
    run(&url, "SET SQL_MODE=@OLD_SQL_MODE").expect("restoring sql_mode must not fail with 1231");
}
