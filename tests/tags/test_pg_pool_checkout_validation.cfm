<cfscript>
// PostgreSQL connection pool must NOT validate (ping) on every checkout
// (Lucee parity / remote-DB performance).
//
// Lucee's JDBC pool does not run a liveness query on checkout by default
// (datasource `validate` defaults off), so a `cfquery` is ONE round-trip to
// the server. RustCFML's pool (r2d2 PostgresConnectionManager) leaves r2d2's
// `test_on_check_out` at its default (true) and implements is_valid as
// `simple_query("SELECT 1")` — so EVERY pooled checkout fires an extra
// `SELECT 1` before the real statement. Because a connection is checked out
// per query (execute_postgres -> pool.get() per cfquery), each cfquery costs
// TWO round-trips. Against a remote DB (e.g. Neon) this ~doubles latency vs
// Lucee on the same database.
//
// Engine fix is one line in get_postgres_pool's builder:
//   r2d2::Pool::builder() ... .test_on_check_out(false)
// (lean on has_broken()/retry for dead connections instead of pinging every
// checkout). See crates/cfml-stdlib/src/builtins.rs: is_valid (~6526) and the
// pool builder (~6719).
//
// Observed via pg_stat_statements: after a stats reset, a single non-trivial
// cfquery should leave ZERO bare `SELECT 1` (normalized `SELECT $1`) calls
// recorded. Today RustCFML records one per checkout.
//
// Live test, gated on RUSTCFML_TEST_PG_URL, and additionally skipped unless
// pg_stat_statements is available (it must be preloaded). To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16 \
//     -c shared_preload_libraries=pg_stat_statements
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL pool checkout validation (skipped without RUSTCFML_TEST_PG_URL + pg_stat_statements)");

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped — set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("SELECT 1", [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped — PostgreSQL not reachable: " & e.message & ")" & chr(10));
    }
}

// pg_stat_statements must be preloaded (shared_preload_libraries); if we can't
// create it, skip rather than fail — the gap is a perf one, not a correctness
// one, and this is the only CFML-observable signal for it.
if (NOT pgskip) {
    try {
        queryExecute("CREATE EXTENSION IF NOT EXISTS pg_stat_statements", [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped — pg_stat_statements unavailable; start postgres with "
            & "-c shared_preload_libraries=pg_stat_statements: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    // Clear stats, then run exactly one structurally-distinct marker query
    // (two columns -> normalizes to `SELECT $1 AS ..., $2 AS ...`, which can
    // never collide with the pool's bare `SELECT 1` -> `SELECT $1`).
    queryExecute("SELECT pg_stat_statements_reset()", [], { datasource: pgdsn });
    queryExecute("SELECT 100 AS zz_pool_marker, 200 AS zz_pool_marker2", [], { datasource: pgdsn });

    // Count recorded bare-`SELECT 1` validation pings. Match both the
    // normalized (`SELECT $1`) and, defensively, un-normalized (`SELECT 1`)
    // forms. Any such call after the reset can only come from the pool's
    // per-checkout is_valid — the test issues no bare `SELECT 1` itself.
    q = queryExecute(
        "SELECT COALESCE(SUM(calls), 0)::int AS n FROM pg_stat_statements "
        & "WHERE query IN ('SELECT $1', 'SELECT 1')",
        [],
        { datasource: pgdsn }
    );
    validationPings = q.n[1];

    assertTrue(
        "PG pool must not ping SELECT 1 on checkout (Lucee parity); recorded "
        & validationPings & " validation query call(s) after one cfquery",
        validationPings == 0
    );
}

suiteEnd();
</cfscript>
