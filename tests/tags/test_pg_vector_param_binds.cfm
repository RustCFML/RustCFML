<cfscript>
// PostgreSQL pgvector parameter binds (Lucee parity).
//
// Lucee binds vector-literal text parameters (e.g. "[1,0,0]") to pgvector
// `vector` columns. RustCFML binds parameters in the binary wire format;
// extension types like `vector` have no static OID (they must be matched by
// type NAME) and pgvector's binary format is: uint16 dimension count,
// uint16 flags, then IEEE-754 big-endian f32 per element. Sending raw text
// fails server-side, surfaced as a generic "db error". This breaks any app
// doing embedding INSERTs or nearest-neighbour ORDER BY with a bound vector.
//
// Live test, gated on RUSTCFML_TEST_PG_URL. Additionally self-skips when the
// `vector` extension is not installed on the server. To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres pgvector/pgvector:pg16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL pgvector param binds (skipped without RUSTCFML_TEST_PG_URL)");

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;
tbl = "zz_rcfml_vector_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped — set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("CREATE EXTENSION IF NOT EXISTS vector", [], { datasource: pgdsn });
        queryExecute("CREATE TABLE #tbl# (id int, v vector(3))", [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped — PostgreSQL/pgvector not available: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    try {
        // --- gap: vector-literal text bound to a vector column ---
        queryExecute("INSERT INTO #tbl# (id, v) VALUES (?, ?)",
            [1, "[1,0,0]"], { datasource: pgdsn });
        queryExecute("INSERT INTO #tbl# (id, v) VALUES (?, ?)",
            [2, "[0,1,0]"], { datasource: pgdsn });

        r1 = queryExecute("SELECT v::text AS vt FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("vector param round-trips", r1.vt[1] ?: "", "[1,0,0]");

        // --- gap: bound vector param in a nearest-neighbour ORDER BY ---
        r2 = queryExecute("SELECT id FROM #tbl# ORDER BY v <-> ? LIMIT 1",
            ["[0.9,0.1,0]"], { datasource: pgdsn });
        assert("vector param in <-> ORDER BY finds nearest", r2.id[1] ?: 0, 1);

        // --- gap: distance of identical vectors is zero ---
        r3 = queryExecute("SELECT (v <-> ?) AS d FROM #tbl# WHERE id = 2",
            ["[0,1,0]"], { datasource: pgdsn });
        assert("vector param distance to itself", r3.d[1] ?: -1, 0);
    } catch (any e) {
        assertTrue("pgvector binds failed: " & e.message, false);
    }

    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: pgdsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
