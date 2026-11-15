<cfscript>
// PostgreSQL extended-type parameter binds (Lucee/BoxLang parity).
//
// rust-postgres sends every parameter in the BINARY wire format, whereas
// Lucee and BoxLang (both pgjdbc) send parameters as TEXT and let the server
// parse them. RustCFML binary-encodes the common scalar types; for everything
// else — arrays, interval, inet/cidr, macaddr, timetz, ranges, … — the bind
// value is sent in TEXT format so the server parses it, matching the JDBC
// engines. Without that, these all failed with "incorrect binary data format"
// (surfaced as a generic "db error"). This is the same root cause as the
// temporal / jsonb / pgvector binds; this file guards the long tail.
//
// Live test, gated on RUSTCFML_TEST_PG_URL (same pattern as the S3 tests).
// To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL extended param binds (skipped without RUSTCFML_TEST_PG_URL)");

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;
tbl = "zz_rcfml_ext_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped — set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("CREATE TABLE #tbl# (id int, ai int[], at text[], iv interval, ip inet, mac macaddr, tz timetz)",
            [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped — PostgreSQL not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    try {
        queryExecute("INSERT INTO #tbl# (id) VALUES (1)", [], { datasource: pgdsn });

        // --- int[] array literal text bound to an int[] column ---
        queryExecute("UPDATE #tbl# SET ai = ? WHERE id = 1", ["{10,20,30}"], { datasource: pgdsn });
        r1 = queryExecute("SELECT ai::::text AS s, ai[2] AS second FROM #tbl# WHERE id = 1", [], { datasource: pgdsn });
        assert("int[] param round-trips", r1.s[1] ?: "", "{10,20,30}");
        assert("int[] element addressable server-side", r1.second[1] ?: 0, 20);

        // --- text[] array ---
        queryExecute("UPDATE #tbl# SET at = ? WHERE id = 1", ["{alpha,beta}"], { datasource: pgdsn });
        r2 = queryExecute("SELECT array_length(at, 1) AS n FROM #tbl# WHERE id = 1", [], { datasource: pgdsn });
        assert("text[] param keeps length", r2.n[1] ?: 0, 2);

        // --- interval ---
        queryExecute("UPDATE #tbl# SET iv = ? WHERE id = 1", ["2 days 03:00:00"], { datasource: pgdsn });
        r3 = queryExecute("SELECT iv::::text AS s FROM #tbl# WHERE id = 1", [], { datasource: pgdsn });
        assert("interval param round-trips", r3.s[1] ?: "", "2 days 03:00:00");

        // --- inet (server canonicalises to /32) ---
        queryExecute("UPDATE #tbl# SET ip = ? WHERE id = 1", ["10.0.0.5"], { datasource: pgdsn });
        r4 = queryExecute("SELECT host(ip) AS h FROM #tbl# WHERE id = 1", [], { datasource: pgdsn });
        assert("inet param round-trips", r4.h[1] ?: "", "10.0.0.5");

        // --- macaddr ---
        queryExecute("UPDATE #tbl# SET mac = ? WHERE id = 1", ["08:00:2b:01:02:03"], { datasource: pgdsn });
        r5 = queryExecute("SELECT mac::::text AS s FROM #tbl# WHERE id = 1", [], { datasource: pgdsn });
        assert("macaddr param round-trips", r5.s[1] ?: "", "08:00:2b:01:02:03");

        // --- timetz (rendered via ::text; to_char has no timetz overload) ---
        queryExecute("UPDATE #tbl# SET tz = ? WHERE id = 1", ["10:30:45+02"], { datasource: pgdsn });
        r6 = queryExecute("SELECT tz::::text AS s FROM #tbl# WHERE id = 1", [], { datasource: pgdsn });
        assert("timetz param binds", r6.s[1] ?: "", "10:30:45+02");

        // --- array param in an expression (= ANY) ---
        r7 = queryExecute("SELECT count(*) AS c FROM #tbl# WHERE 20 = ANY(ai)", [], { datasource: pgdsn });
        assert("int[] usable in = ANY()", r7.c[1] ?: 0, 1);
    } catch (any e) {
        assertTrue("extended binds failed: " & e.message, false);
    }

    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: pgdsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
