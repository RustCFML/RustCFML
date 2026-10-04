//! Live-MySQL regression test for connection checkout without a per-checkout PING.
//!
//! The pool no longer PINGs every connection it hands out (one extra round-trip on
//! the first query of every request; Lucee does not validate on borrow). A pooled
//! connection returned within a short window is used as-is, so one the server has
//! killed in the meantime must not poison the pool:
//! * a READ on such a connection is retried once on another connection;
//! * a WRITE fails (it is never retried), and the dead connection is discarded so
//!   the next statement succeeds.
//!
//! Gated on `RUSTCFML_MYSQL_TEST_URL` (a no-op otherwise), e.g.
//!
//! ```bash
//! RUSTCFML_MYSQL_TEST_URL='mysql://root:pw@127.0.0.1:3306/db' \
//!   cargo test -p cfml-stdlib --features all-databases --test mysql_dead_connection_recovery -- --test-threads=1
//! ```

#![cfg(feature = "mysql_db")]

use cfml_common::dynamic::{CfmlValue, ValueMap};
use mysql::prelude::Queryable;

fn run(url: &str, sql: &str) -> cfml_common::vm::CfmlResult {
    let mut options = ValueMap::default();
    options.insert("datasource".to_string(), CfmlValue::string(url.to_string()));
    cfml_stdlib::fn_query_execute(vec![
        CfmlValue::string(sql.to_string()),
        CfmlValue::Null,
        CfmlValue::strukt(options),
    ])
}

/// The server connection id the next statement on `url` runs on, after ending the
/// "request" so the connection goes back to the pool with a fresh use stamp.
fn pooled_connection_id(url: &str) -> String {
    let r = run(url, "SELECT CONNECTION_ID() AS id").expect("connection id");
    cfml_stdlib::release_request_db_conns();
    let s = format!("{:?}", r);
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn kill(url: &str, id: &str) {
    let mut c = mysql::Conn::new(mysql::Opts::from_url(url).unwrap()).expect("killer connection");
    c.query_drop(format!("KILL {}", id)).expect("KILL");
}

#[test]
fn killed_pooled_connection_is_discarded_and_reads_retry() {
    let Ok(url) = std::env::var("RUSTCFML_MYSQL_TEST_URL") else {
        eprintln!("skipping: RUSTCFML_MYSQL_TEST_URL not set");
        return;
    };

    // Read straight after the kill: retried transparently.
    let id = pooled_connection_id(&url);
    kill(&url, &id);
    run(&url, "SELECT 1 AS x").expect("a read on a killed pooled connection is retried");
    cfml_stdlib::release_request_db_conns();

    // Write straight after the kill: fails, is NOT retried (it was not applied) ...
    run(&url, "DROP TABLE IF EXISTS rustcfml_deadconn_probe").unwrap();
    run(&url, "CREATE TABLE rustcfml_deadconn_probe (n INT)").unwrap();
    cfml_stdlib::release_request_db_conns();
    let id = pooled_connection_id(&url);
    kill(&url, &id);
    assert!(
        run(&url, "INSERT INTO rustcfml_deadconn_probe VALUES (1)").is_err(),
        "a write on a killed connection must surface the error, not be retried"
    );
    cfml_stdlib::release_request_db_conns();
    // ... and the dead connection did not go back to the pool.
    for _ in 0..3 {
        let r = run(&url, "SELECT COUNT(*) AS n FROM rustcfml_deadconn_probe")
            .expect("pool must not hand out the killed connection again");
        assert!(format!("{:?}", r).contains('0'), "the failed write was not applied: {:?}", r);
        cfml_stdlib::release_request_db_conns();
    }
    run(&url, "DROP TABLE rustcfml_deadconn_probe").unwrap();
    cfml_stdlib::release_request_db_conns();
}
