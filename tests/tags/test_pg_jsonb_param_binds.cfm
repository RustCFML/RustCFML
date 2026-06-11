<cfscript>
// PostgreSQL jsonb/json parameter binds (Lucee parity).
//
// Lucee binds JSON text parameters to PostgreSQL jsonb and json columns.
// RustCFML binds parameters in the binary wire format; jsonb's binary format
// requires a 1-byte version prefix (0x01) before the JSON text. Sending the
// raw text fails server-side with "unsupported jsonb version number 123"
// (123 is the byte value of '{'), surfaced as a generic "db error".
//
// Live test, gated on RUSTCFML_TEST_PG_URL (same pattern as the S3 tests).
// To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL jsonb/json param binds (skipped without RUSTCFML_TEST_PG_URL)");

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;
tbl = "zz_rcfml_jsonb_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped — set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("CREATE TABLE #tbl# (id int, jb jsonb, j json)",
            [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped — PostgreSQL not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    try {
        payload = serializeJSON({ "name": "alpha", "tags": ["x", "y"] });

        // --- gap: JSON text bound to a jsonb column ---
        queryExecute("INSERT INTO #tbl# (id, jb) VALUES (?, ?)",
            [1, payload], { datasource: pgdsn });
        r1 = queryExecute("SELECT jb->>'name' AS n, jsonb_array_length(jb->'tags') AS c FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("jsonb param round-trips (->> extraction)", r1.n[1] ?: "", "alpha");
        assert("jsonb param keeps array structure", r1.c[1] ?: 0, 2);

        // --- gap: JSON text bound to a json column ---
        queryExecute("INSERT INTO #tbl# (id, j) VALUES (?, ?)",
            [2, payload], { datasource: pgdsn });
        r2 = queryExecute("SELECT j->>'name' AS n FROM #tbl# WHERE id = 2",
            [], { datasource: pgdsn });
        assert("json param round-trips (->> extraction)", r2.n[1] ?: "", "alpha");

        // --- gap: jsonb param in an expression (containment test) ---
        r3 = queryExecute("SELECT count(*) AS c FROM #tbl# WHERE jb @> ?::jsonb",
            [serializeJSON({ "name": "alpha" })], { datasource: pgdsn });
        assert("jsonb containment param binds", r3.c[1] ?: 0, 1);
    } catch (any e) {
        assertTrue("jsonb/json binds failed: " & e.message, false);
    }

    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: pgdsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
