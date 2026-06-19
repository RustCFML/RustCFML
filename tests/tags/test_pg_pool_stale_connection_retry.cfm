<cfscript>
// PostgreSQL pooled connections must recover when the server closes an idle
// session behind the pool (Neon scale-to-zero / suspend-resume parity).
//
// RustCFML deliberately does not ping SELECT 1 on every checkout, matching
// Lucee's default datasource behaviour and avoiding an extra remote round-trip.
// Without a one-shot retry, though, a pooled connection whose backend was
// closed while idle is handed out and the first real query fails at prepare
// time with "connection closed".
//
// Live test, gated on RUSTCFML_TEST_PG_URL. To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL stale pooled connection retry (skipped without RUSTCFML_TEST_PG_URL)");

function pgStaleRetryAppendDsnParam(required string dsn, required string key, required string value) {
    return arguments.dsn & (find("?", arguments.dsn) ? "&" : "?") & arguments.key & "=" & arguments.value;
}

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped - set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("SELECT 1", [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped - PostgreSQL not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    try {
        // First checkout creates and returns an idle pooled connection.
        original = queryExecute("SELECT pg_backend_pid() AS pid", [], { datasource: pgdsn });
        originalPid = original.pid[1];

        // A distinct datasource string forces a separate pool/connection, so it
        // can terminate the idle backend sitting in the original pool.
        killerDsn = pgStaleRetryAppendDsnParam(pgdsn, "application_name", "rustcfml_stale_retry_killer");
        killed = queryExecute("SELECT pg_terminate_backend(?::int) AS killed", [originalPid], { datasource: killerDsn });
        assertTrue("test should terminate the original pooled backend", killed.killed[1]);

        // The first attempt receives the stale pooled connection. The runtime
        // must mark it broken, discard it, and retry on a fresh connection.
        retried = queryExecute("SELECT 42 AS n", [], { datasource: pgdsn });
        assert("stale pooled connection is retried transparently", retried.n[1], 42);
    } catch (any e) {
        assertTrue("stale pooled connection retry failed: " & e.message, false);
    }
}

suiteEnd();
</cfscript>
