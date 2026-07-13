//! Live-MySQL regression test for the queryExecute `timeout` option
//! (JDBC setQueryTimeout equivalent — a KILL QUERY watchdog aborts an
//! overrunning statement). Gated on `RUSTCFML_MYSQL_TEST_URL`, so it is a
//! no-op in normal `cargo test` runs and only exercises a real server when
//! that env var points at one, e.g.
//!
//! ```bash
//! RUSTCFML_MYSQL_TEST_URL='mysql://root:freeze@127.0.0.1:3306/preside_test' \
//!   cargo test -p cfml-stdlib --features all-databases --test mysql_query_timeout
//! ```
//!
//! Covers the same scenario as Preside's SqlRunnerTest test05/test06.

#![cfg(feature = "mysql_db")]

use cfml_common::dynamic::{CfmlValue, ValueMap};
use std::time::{Duration, Instant};

#[test]
fn select_sleep_is_aborted_when_timeout_exceeded() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    let mut options = ValueMap::default();
    options.insert("datasource".to_string(), CfmlValue::string(url));
    options.insert("timeout".to_string(), CfmlValue::Int(1));

    let args = vec![
        CfmlValue::string("select sleep( 10 )"),
        CfmlValue::Null,
        CfmlValue::strukt(options),
    ];

    let start = Instant::now();
    let result = cfml_stdlib::fn_query_execute(args);
    let elapsed = start.elapsed();

    // The watchdog must abort well before the 10s the statement would sleep.
    assert!(
        elapsed < Duration::from_secs(5),
        "query should have been cancelled by the timeout, but ran for {:?}",
        elapsed
    );

    let err = result.expect_err("a timed-out query must return an error");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("timeout"),
        "error message should mention the timeout, got: {}",
        msg
    );
}
