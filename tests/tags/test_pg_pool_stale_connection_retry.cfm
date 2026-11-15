<cfscript>
// PostgreSQL pooled connections must recover when the server closes a session
// behind the pool (Neon scale-to-zero / suspend-resume, failover, or a
// pg_terminate_backend / smart shutdown).
//
// RustCFML deliberately does not ping SELECT 1 on every checkout, matching
// Lucee's default datasource behaviour and avoiding an extra remote round-trip.
// A pooled connection whose backend died while idle is therefore handed out and
// the first real query fails — either with a closed socket (is_closed) or a
// server FATAL ("terminating connection due to administrator command"). The
// runtime must mark it broken, discard it, and retry the (side-effect-free)
// statement on a fresh connection.
//
// A single retry is NOT enough: min_idle keeps a spare healthy connection that
// masks the first kill, and when several sessions die at once the pool holds
// multiple dead connections that r2d2 replaces asynchronously. This test kills
// the connection it just used on each round, so by the second round the pool
// must drain a genuinely dead connection — which only succeeds with the bounded
// drain-and-retry loop plus FATAL-state detection.
//
// Live test, gated on RUSTCFML_TEST_PG_URL. To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL stale pooled connection retry (skipped without RUSTCFML_TEST_PG_URL)");

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
        // A distinct datasource string forces a separate pool/connection, so it
        // can terminate the idle backend sitting in the original pool.
        killerDsn = pgdsn & (find("?", pgdsn) ? "&" : "?") & "application_name=rustcfml_stale_retry_killer";

        // Each round: use a connection, terminate that exact backend, then query
        // again. By round 2 the pool is forced to hand back the now-dead
        // connection, so recovery exercises the drain-and-retry path rather than
        // a spare healthy connection.
        recovered = 0;
        for (round = 1; round <= 3; round++) {
            before = queryExecute("SELECT pg_backend_pid() AS pid", [], { datasource: pgdsn });
            queryExecute("SELECT pg_terminate_backend(?::::int)", [before.pid[1]], { datasource: killerDsn });
            after = queryExecute("SELECT 40 + " & round & " AS n", [], { datasource: pgdsn });
            if (after.n[1] == 40 + round) recovered++;
        }
        assert("every query recovers after its backend is terminated", recovered, 3);
    } catch (any e) {
        assertTrue("stale pooled connection retry failed: " & e.message, false);
    }
}

suiteEnd();
</cfscript>
