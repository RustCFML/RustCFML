<cfscript>
// PostgreSQL error messages must surface the server's cause (Lucee parity).
//
// When a query fails server-side, Lucee surfaces the driver's message in
// cfcatch.message (e.g. "function zz_x() does not exist"). RustCFML currently
// reports only the top-level error Display — a bare "db error" — and drops
// the chained source() detail, which makes real failures undiagnosable from
// CFML (and from the server log).
//
// Live test, gated on RUSTCFML_TEST_PG_URL (same pattern as the S3 tests).
// To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run -- tests/runner.cfm

suiteBegin("PostgreSQL error cause chain (skipped without RUSTCFML_TEST_PG_URL)");

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

if (NOT pgskip) {
    // --- gap: a server-side error must carry the server's message ---
    msg = "";
    try {
        queryExecute("SELECT zz_definitely_not_a_function_98765()", [], { datasource: pgdsn });
        assertTrue("undefined-function query should have thrown", false);
    } catch (any e) {
        msg = e.message & " " & (e.detail ?: "");
    }
    assertTrue("error message names the failing function (got: [" & msg & "])",
        find("zz_definitely_not_a_function_98765", msg) GT 0);
    assertTrue("error message carries the server cause, not just 'db error' (got: [" & msg & "])",
        findNoCase("does not exist", msg) GT 0);

    // --- gap: same for a constraint violation ---
    tbl = "zz_rcfml_chain_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));
    msg2 = "";
    try {
        queryExecute("CREATE TABLE #tbl# (id int PRIMARY KEY)", [], { datasource: pgdsn });
        queryExecute("INSERT INTO #tbl# VALUES (1)", [], { datasource: pgdsn });
        queryExecute("INSERT INTO #tbl# VALUES (1)", [], { datasource: pgdsn });
        assertTrue("duplicate-key insert should have thrown", false);
    } catch (any e) {
        msg2 = e.message & " " & (e.detail ?: "");
    }
    assertTrue("constraint violation surfaces the server cause (got: [" & msg2 & "])",
        findNoCase("duplicate key", msg2) GT 0 OR findNoCase("unique", msg2) GT 0);

    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: pgdsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
