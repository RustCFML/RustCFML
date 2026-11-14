<cfscript>
// SQL Server DML with an OUTPUT clause returns rows and must be handled as
// row-returning SQL (the analogue of PostgreSQL's RETURNING, fixed in the PG
// path by GitHub #174).
//
// tiberius' `execute()` does NOT error on a row-returning statement; it counts
// affected rows and silently DISCARDS the OUTPUT result set. So before the fix,
// `INSERT/UPDATE/DELETE ... OUTPUT inserted.*` lost its rows and the caller saw
// a bare mutation result instead of the data Lucee returns. The fix routes
// these through `query`. Note: `OUTPUT ... INTO @tbl` does NOT stream rows back
// to the client, so it must stay on the mutation path.
//
// Live test, gated on RUSTCFML_TEST_MSSQL_DS (same pattern as the other MSSQL
// tests). To run locally:
//   docker run --rm -d -p 1433:1433 -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD=Str0ng!Passw0rd \
//     --platform linux/amd64 mcr.microsoft.com/mssql/server:2022-latest
//   RUSTCFML_TEST_MSSQL_DS=mssql://sa:Str0ng!Passw0rd@127.0.0.1:1433/master \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("SQL Server DML OUTPUT rows (skipped without RUSTCFML_TEST_MSSQL_DS)");

mssqldsn = "";
try {
    mssqldsn = server.system.environment.RUSTCFML_TEST_MSSQL_DS ?: "";
} catch (any e) {
    mssqldsn = "";
}

mssqlskip = false;
// `####` is SQL Server's `##` (a global temp table, visible to every pooled
// connection): `##` in a CFML string is one escaped `#`, a connection-local
// temp table the next pooled statement cannot see.
tbl = "####zz_rcfml_output_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (mssqldsn == "") {
    mssqlskip = true;
    writeOutput("  (skipped - set RUSTCFML_TEST_MSSQL_DS to enable)" & chr(10));
} else {
    try {
        // Temp table (## = global temp) so we don't require CREATE TABLE in a
        // shared schema; lives for the connection/session lifetime.
        queryExecute("CREATE TABLE #tbl# (id int PRIMARY KEY, label nvarchar(50), touched bit DEFAULT 0)",
            [], { datasource: mssqldsn });
        queryExecute("INSERT INTO #tbl# (id, label) VALUES (1, 'alpha'), (2, 'beta')",
            [], { datasource: mssqldsn });
    } catch (any e) {
        mssqlskip = true;
        writeOutput("  (skipped - SQL Server not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT mssqlskip) {
    try {
        rows = queryExecute(
            "UPDATE #tbl# SET touched = 1 OUTPUT inserted.id, inserted.label, inserted.touched WHERE id = ?",
            [1],
            { datasource: mssqldsn, returntype: "array" }
        );
        assertTrue("queryExecute UPDATE OUTPUT returns an array", isArray(rows));
        assert("queryExecute UPDATE OUTPUT row count", arrayLen(rows), 1);
        assert("queryExecute UPDATE OUTPUT id", rows[1].id ?: 0, 1);
        assert("queryExecute UPDATE OUTPUT label", rows[1].label ?: "", "alpha");

        none = queryExecute(
            "UPDATE #tbl# SET touched = 1 OUTPUT inserted.id WHERE id = ?",
            [999],
            { datasource: mssqldsn, returntype: "array" }
        );
        assertTrue("queryExecute UPDATE OUTPUT no-match still returns array", isArray(none));
        assert("queryExecute UPDATE OUTPUT no-match row count", arrayLen(none), 0);

        inserted = queryExecute(
            "INSERT INTO #tbl# (id, label) OUTPUT inserted.id, inserted.label VALUES (?, ?)",
            [3, "gamma"],
            { datasource: mssqldsn, returntype: "array" }
        );
        assert("queryExecute INSERT OUTPUT row count", arrayLen(inserted), 1);
        assert("queryExecute INSERT OUTPUT id", inserted[1].id ?: 0, 3);
        assert("queryExecute INSERT OUTPUT label", inserted[1].label ?: "", "gamma");

        deleted = queryExecute(
            "DELETE FROM #tbl# OUTPUT deleted.label WHERE id = ?",
            [2],
            { datasource: mssqldsn, returntype: "array" }
        );
        assert("queryExecute DELETE OUTPUT row count", arrayLen(deleted), 1);
        assert("queryExecute DELETE OUTPUT label", deleted[1].label ?: "", "beta");

        // OUTPUT ... INTO @tablevar does NOT stream rows to the client; it must
        // stay on the mutation path and report rows affected, not return rows.
        // Like any DML, it returns an empty query; the count is in result=.
        intoResult = queryExecute(
            "DECLARE @captured TABLE (id int);
             UPDATE #tbl# SET touched = 0 OUTPUT inserted.id INTO @captured WHERE id = ?;",
            [1],
            { datasource: mssqldsn, result: "intoMeta" }
        );
        assertTrue("queryExecute UPDATE OUTPUT INTO streams no rows",
            isQuery(intoResult) AND intoResult.recordCount == 0);
        assert("queryExecute UPDATE OUTPUT INTO reports the rows updated", intoMeta.recordCount, 1);
    } catch (any e) {
        assertTrue("SQL Server queryExecute DML OUTPUT failed: " & e.message, false);
    }

    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: mssqldsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
