<cfscript>
// PostgreSQL temporal parameter binds (Lucee parity).
//
// Lucee binds CFML date/time values (and date-like strings) to PostgreSQL
// timestamp / timestamptz / date / time columns. RustCFML currently binds
// every parameter in the binary wire format but has no temporal encodings,
// so any parameter bound to a temporal column fails server-side with
// "incorrect binary data format in bind parameter N" (surfaced as a generic
// "db error").
//
// Live test, gated on RUSTCFML_TEST_PG_URL (same pattern as the S3 tests).
// To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL temporal param binds (skipped without RUSTCFML_TEST_PG_URL)");

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;
tbl = "zz_rcfml_temporal_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped — set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("CREATE TABLE #tbl# (id int, ts timestamptz, tsn timestamp, d date, t time)",
            [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped — PostgreSQL not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    try {
        // --- control: non-temporal binds already work ---
        queryExecute("INSERT INTO #tbl# (id) VALUES (?)", [1], { datasource: pgdsn });
        rc = queryExecute("SELECT id FROM #tbl# WHERE id = ?", [1], { datasource: pgdsn });
        assert("integer param binds (control)", rc.recordCount, 1);

        // --- gap: CFML datetime value bound to timestamptz ---
        queryExecute("UPDATE #tbl# SET ts = ? WHERE id = 1",
            [createDateTime(2024, 3, 15, 10, 30, 45)], { datasource: pgdsn });
        r1 = queryExecute("SELECT to_char(ts, 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("datetime value binds to timestamptz", r1.s[1] ?: "", "2024-03-15 10:30:45");

        // --- gap: CFML datetime value bound to timestamp (no tz) ---
        queryExecute("UPDATE #tbl# SET tsn = ? WHERE id = 1",
            [createDateTime(2025, 12, 31, 23, 59, 59)], { datasource: pgdsn });
        r2 = queryExecute("SELECT to_char(tsn, 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("datetime value binds to timestamp", r2.s[1] ?: "", "2025-12-31 23:59:59");

        // --- gap: CFML date value bound to date ---
        queryExecute("UPDATE #tbl# SET d = ? WHERE id = 1",
            [createDate(2024, 3, 15)], { datasource: pgdsn });
        r3 = queryExecute("SELECT to_char(d, 'YYYY-MM-DD') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("date value binds to date", r3.s[1] ?: "", "2024-03-15");

        // --- gap: CFML time value bound to time ---
        queryExecute("UPDATE #tbl# SET t = ? WHERE id = 1",
            [createTime(10, 30, 45)], { datasource: pgdsn });
        r4 = queryExecute("SELECT to_char(t, 'HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("time value binds to time", r4.s[1] ?: "", "10:30:45");

        // --- gap: date-like STRING bound to timestamp (Lucee accepts) ---
        queryExecute("UPDATE #tbl# SET tsn = ? WHERE id = 1",
            ["2026-01-02 03:04:05"], { datasource: pgdsn });
        r5 = queryExecute("SELECT to_char(tsn, 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("ISO string binds to timestamp", r5.s[1] ?: "", "2026-01-02 03:04:05");

        // --- gap: ISO 8601 / RFC 3339 strings WITH a timezone offset or
        //     fractional seconds bind to timestamptz. This is the exact shape
        //     RustCFML serializes a timestamptz column TO in JSON output
        //     ("2026-06-10T07:20:42.177+00:00"), so an app that reads a record
        //     and saves it back must be able to re-bind the engine's own
        //     emitted value. ---
        queryExecute("UPDATE #tbl# SET ts = ? WHERE id = 1",
            ["2026-06-10T07:20:42+00:00"], { datasource: pgdsn });
        r5a = queryExecute("SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("ISO 8601 with offset binds to timestamptz", r5a.s[1] ?: "", "2026-06-10 07:20:42");

        queryExecute("UPDATE #tbl# SET ts = ? WHERE id = 1",
            ["2026-06-10T07:20:42.177+00:00"], { datasource: pgdsn });
        r5b = queryExecute("SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("ISO 8601 with fractional seconds + offset binds to timestamptz", r5b.s[1] ?: "", "2026-06-10 07:20:42");

        queryExecute("UPDATE #tbl# SET ts = ? WHERE id = 1",
            ["2026-06-10T07:20:42.177Z"], { datasource: pgdsn });
        r5c = queryExecute("SELECT to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("ISO 8601 with Z (Zulu) suffix binds to timestamptz", r5c.s[1] ?: "", "2026-06-10 07:20:42");

        queryExecute("UPDATE #tbl# SET tsn = ? WHERE id = 1",
            ["2026-06-10T07:20:42.177"], { datasource: pgdsn });
        r5d = queryExecute("SELECT to_char(tsn, 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("ISO 8601 'T' with fractional seconds (no tz) binds to timestamp", r5d.s[1] ?: "", "2026-06-10 07:20:42");

        // --- gap: cfqueryparam-style struct with cf_sql_timestamp ---
        queryExecute("UPDATE #tbl# SET ts = ? WHERE id = 1",
            [{ value: createDateTime(2024, 6, 1, 12, 0, 0), cfsqltype: "cf_sql_timestamp" }],
            { datasource: pgdsn });
        r6 = queryExecute("SELECT to_char(ts, 'YYYY-MM-DD HH24:MI:SS') AS s FROM #tbl# WHERE id = 1",
            [], { datasource: pgdsn });
        assert("cf_sql_timestamp param struct binds", r6.s[1] ?: "", "2024-06-01 12:00:00");
    } catch (any e) {
        assertTrue("temporal binds failed: " & e.message, false);
    }

    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: pgdsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
