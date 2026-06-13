<cfscript>
// A `WITH` (CTE) statement is a row-returning query (Lucee parity).
//
// CTEs are standard SQL; Lucee runs `WITH … SELECT …` as an ordinary query
// and returns its rows. RustCFML decides whether a statement returns rows by
// looking for a leading `SELECT`, so a `WITH` prefix is misclassified as a
// non-row "execute" statement:
//   - SQLite: errors "Execute returned results - did you mean to call query?"
//   - PostgreSQL: the row is counted but every column reads back NULL.
//
// Real-world hit: the Moopa sysadmin route-security panel builds its payload
// with `WITH profile_subjects AS (…), role_subjects AS (…) SELECT
// row_to_json(…)::text AS data …`; the `data` column comes back empty and
// `deserializeJSON("")` 500s the endpoint.
//
// Self-contained via the bundled SQLite driver (same skip pattern as
// tests/tags/test_queryexecute_param_structs.cfm).

suiteBegin("queryExecute: WITH (CTE) statements are row-returning queries");

tmpfile = getTempDirectory() & "/rustcfml_cte_" & createUUID() & ".sqlite";
ds = "sqlite://" & tmpfile;
skip = false;

try {
    queryExecute("CREATE TABLE t (id INTEGER, label TEXT)", [], { datasource: ds });
    queryExecute("INSERT INTO t VALUES (1, 'alpha'), (2, 'beta')", [], { datasource: ds });
} catch (any e) {
    skip = true;
    assertTrue("CTE test skipped (no sqlite): " & e.message, true);
}
</cfscript>

<cfif NOT skip>
<cfscript>
// --- control: a plain SELECT returns rows today ---
ctrl = queryExecute("SELECT id, label FROM t ORDER BY id", [], { datasource: ds });
assert("control: plain SELECT returns rows", ctrl.recordCount, 2);

// --- gap: an inline-VALUES CTE must return its rows ---
try {
    rValues = queryExecute(
        "WITH cte(n) AS (VALUES (10), (20), (30)) SELECT n FROM cte ORDER BY n",
        [], { datasource: ds });
    assert("inline-VALUES CTE returns its rows", rValues.recordCount, 3);
    assert("inline-VALUES CTE first value", rValues.n[1] ?: "", "10");
} catch (any e) {
    assertTrue("inline-VALUES CTE query failed (misclassified as execute?): " & e.message, false);
}

// --- gap: a CTE over a real table must return its rows AND values ---
try {
    rTable = queryExecute(
        "WITH labelled AS (SELECT id, label FROM t) SELECT label FROM labelled ORDER BY id",
        [], { datasource: ds });
    assert("table CTE returns its rows", rTable.recordCount, 2);
    // the PostgreSQL form of this bug returns the row but a NULL column, so
    // assert the actual value, not just the row count
    assert("table CTE first column value is not lost", rTable.label[1] ?: "", "alpha");
} catch (any e) {
    assertTrue("table CTE query failed (misclassified as execute?): " & e.message, false);
}

// --- gap: a CTE feeding an aggregate (the shape the Moopa panel uses) ---
try {
    rAgg = queryExecute(
        "WITH labelled AS (SELECT label FROM t) SELECT count(*) AS c FROM labelled",
        [], { datasource: ds });
    assert("CTE feeding an aggregate returns the value", rAgg.c[1] ?: "", "2");
} catch (any e) {
    assertTrue("CTE-with-aggregate query failed: " & e.message, false);
}

try { queryExecute("DROP TABLE t", [], { datasource: ds }); } catch (any e) {}
</cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
