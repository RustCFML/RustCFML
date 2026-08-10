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

/// GitHub #275: a transient `SET foreign_key_checks=0` must NOT outlive the
/// request. `release_request_db_conns` (called at request boundaries) returns the
/// held connection to the pool, which resets it — so the next request's queries
/// see the default session state, not the leaked one.
#[test]
fn session_state_reset_at_request_boundary() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    // "Request A": disable FK checks, then confirm it took on the held connection.
    run(&url, "SET SESSION foreign_key_checks=0").expect("SET fkc=0 should succeed");
    let within = run(&url, "SELECT @@session.foreign_key_checks AS v")
        .expect("read fkc within request A");
    assert!(
        format!("{:?}", within).contains('0'),
        "within the same request the SET persists (Lucee parity), got: {:?}",
        within
    );

    // Request boundary: release held connections (resets session state).
    cfml_stdlib::builtins::release_request_db_conns();

    // "Request B": the leaked `foreign_key_checks=0` must be gone (back to 1).
    let after = run(&url, "SELECT @@session.foreign_key_checks AS v")
        .expect("read fkc in request B");
    assert!(
        format!("{:?}", after).contains('1'),
        "foreign_key_checks must reset to 1 after the request boundary, got: {:?}",
        after
    );
}

/// A zero-row result set must still expose its column list — the
/// `SELECT * FROM t WHERE 0=1` "give me the columns" idiom. Columns are read from
/// the result-set metadata, not inferred from the first row.
#[test]
fn zero_row_result_keeps_column_list() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    run(&url, "DROP TABLE IF EXISTS _rcf_cols_probe").expect("drop temp table");
    run(
        &url,
        "CREATE TABLE _rcf_cols_probe (alpha INT, beta VARCHAR(10), gamma INT)",
    )
    .expect("create temp table");
    let result = run(&url, "SELECT * FROM _rcf_cols_probe WHERE 0=1")
        .expect("zero-row select should succeed");
    let CfmlValue::Query(q) = &result else {
        panic!("expected a Query, got: {:?}", result);
    };
    assert_eq!(q.row_count(), 0, "probe returns no rows");
    let cols = q.columns().join(",").to_lowercase();
    assert_eq!(
        cols, "alpha,beta,gamma",
        "zero-row result must carry its column list, got: {}",
        cols
    );
    run(&url, "DROP TABLE IF EXISTS _rcf_cols_probe").expect("drop temp table");
}

/// Dirty-tracking counterpart of `session_state_reset_at_request_boundary`: a
/// request that ran ONLY clean SQL (reads/DML) skips COM_RESET_CONNECTION at the
/// request boundary, so the connection's server-side prepared statements survive
/// and the next request's identical statements are cache hits — zero
/// COM_STMT_PREPARE round-trips. (A blanket per-request reset wiped the statement
/// cache, forcing every warm request to re-prepare every statement.)
///
/// Observed via the connection's own `Com_stmt_prepare` session counter. The test
/// uses a URL with an extra opt so it gets a PRIVATE pool (pool key = URL string):
/// with a single serial user, checkout always returns the same connection, making
/// the session counter deterministic even when other tests here run in parallel
/// against the shared pool.
#[test]
fn clean_request_keeps_prepared_statements_across_boundary() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };
    let sep = if url.contains('?') { '&' } else { '?' };
    let url = format!("{url}{sep}prefer_socket=false");

    fn com_stmt_prepare(url: &str) -> i64 {
        // SHOW is classified clean, so this probe never dirties the connection.
        let result = run(url, "SHOW SESSION STATUS LIKE 'Com_stmt_prepare'")
            .expect("SHOW SESSION STATUS should succeed");
        let CfmlValue::Query(q) = &result else {
            panic!("expected a Query, got: {:?}", result);
        };
        let debug = format!("{:?}", q);
        // Row shape: Variable_name = Com_stmt_prepare, Value = <n>. Pull the
        // last integer out of the debug rendering rather than depending on the
        // query accessor surface.
        debug
            .rsplit(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
            .expect("Com_stmt_prepare value should parse")
    }

    // Fresh boundary so this test can't inherit held-conn/dirty state.
    cfml_stdlib::builtins::release_request_db_conns();

    // "Request A": clean statements only.
    run(&url, "SELECT 1 AS v").expect("request A select");
    let before = com_stmt_prepare(&url);
    assert!(before > 0, "request A should have prepared at least one statement");

    // Clean request boundary: the held connection must NOT be reset.
    cfml_stdlib::builtins::release_request_db_conns();

    // "Request B": identical statements — all must hit the surviving cache.
    run(&url, "SELECT 1 AS v").expect("request B select");
    let after = com_stmt_prepare(&url);

    assert_eq!(
        before, after,
        "a clean request boundary must preserve the prepared-statement cache \
         (no new COM_STMT_PREPARE in request B); before={before} after={after}"
    );

    // Leave no held connection behind for other tests.
    cfml_stdlib::builtins::release_request_db_conns();
}
