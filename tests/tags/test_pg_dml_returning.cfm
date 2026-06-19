<cfscript>
// PostgreSQL DML with RETURNING must be handled as row-returning SQL.
//
// The postgres crate rejects `execute()` for statements that return rows:
// "Execute returned results - did you mean to call query?". Lucee returns the
// rows from INSERT/UPDATE/DELETE ... RETURNING, and CFML applications commonly
// depend on that for atomic "claim and return" updates.
//
// Live test, gated on RUSTCFML_TEST_PG_URL (same pattern as the other PG tests).
// To run locally:
//   docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16
//   RUSTCFML_TEST_PG_URL=postgresql://postgres:postgres@127.0.0.1:5432/postgres \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("PostgreSQL DML RETURNING rows (skipped without RUSTCFML_TEST_PG_URL)");

pgdsn = "";
try {
    pgdsn = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
} catch (any e) {
    pgdsn = "";
}

pgskip = false;
tbl = "zz_rcfml_returning_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (pgdsn == "") {
    pgskip = true;
    writeOutput("  (skipped - set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
} else {
    try {
        queryExecute("CREATE TABLE #tbl# (id int PRIMARY KEY, label text, touched boolean default false)",
            [], { datasource: pgdsn });
        queryExecute("INSERT INTO #tbl# (id, label) VALUES (1, 'alpha'), (2, 'beta')",
            [], { datasource: pgdsn });
    } catch (any e) {
        pgskip = true;
        writeOutput("  (skipped - PostgreSQL not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT pgskip) {
    try {
        rows = queryExecute(
            "UPDATE #tbl# SET touched = true WHERE id = ? RETURNING id, label, touched",
            [1],
            { datasource: pgdsn, returntype: "array" }
        );
        assertTrue("queryExecute UPDATE RETURNING returns an array", isArray(rows));
        assert("queryExecute UPDATE RETURNING row count", arrayLen(rows), 1);
        assert("queryExecute UPDATE RETURNING id", rows[1].id ?: 0, 1);
        assert("queryExecute UPDATE RETURNING label", rows[1].label ?: "", "alpha");
        assertTrue("queryExecute UPDATE RETURNING boolean", rows[1].touched ?: false);

        none = queryExecute(
            "UPDATE #tbl# SET touched = true WHERE id = ? RETURNING id",
            [999],
            { datasource: pgdsn, returntype: "array" }
        );
        assertTrue("queryExecute UPDATE RETURNING no-match still returns array", isArray(none));
        assert("queryExecute UPDATE RETURNING no-match row count", arrayLen(none), 0);

        inserted = queryExecute(
            "INSERT INTO #tbl# (id, label) VALUES (?, ?) RETURNING id, label",
            [3, "gamma"],
            { datasource: pgdsn, returntype: "struct" }
        );
        assert("queryExecute INSERT RETURNING id", inserted.id ?: 0, 3);
        assert("queryExecute INSERT RETURNING label", inserted.label ?: "", "gamma");
    } catch (any e) {
        assertTrue("PostgreSQL queryExecute DML RETURNING failed: " & e.message, false);
    }
}
</cfscript>

<cfif NOT pgskip>
    <cftry>
        <cfquery name="tagRows" datasource="#pgdsn#" returntype="array">
            UPDATE #tbl#
            SET touched = false
            WHERE id = <cfqueryparam value="2" cfsqltype="cf_sql_integer">
            RETURNING id, label, touched
        </cfquery>
        <cfscript>
        assertTrue("cfquery UPDATE RETURNING returns an array", isArray(tagRows));
        assert("cfquery UPDATE RETURNING row count", arrayLen(tagRows), 1);
        assert("cfquery UPDATE RETURNING id", tagRows[1].id ?: 0, 2);
        assert("cfquery UPDATE RETURNING label", tagRows[1].label ?: "", "beta");
        assertFalse("cfquery UPDATE RETURNING boolean", tagRows[1].touched ?: true);

        deleted = queryExecute(
            "DELETE FROM #tbl# WHERE id = ? RETURNING label",
            [2],
            { datasource: pgdsn, returntype: "array" }
        );
        assert("queryExecute DELETE RETURNING row count", arrayLen(deleted), 1);
        assert("queryExecute DELETE RETURNING label", deleted[1].label ?: "", "beta");
        </cfscript>
        <cfcatch type="any">
            <cfscript>
            assertTrue("PostgreSQL cfquery DML RETURNING failed: " & cfcatch.message, false);
            </cfscript>
        </cfcatch>
    </cftry>
</cfif>

<cfscript>
if (NOT pgskip) {
    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: pgdsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
