<cfscript>
suiteBegin("queryExecute maxrows option + list:true IN-clause expansion");

// ============================================================
// Gaps surfaced laddering the Wheels framework test suite on RustCFML 0.309.0.
// ============================================================
// Both reproduce in pure Query-of-Queries (dbtype="query"), so they are
// engine-wide query-binding gaps, NOT database-specific. Surfaced by
// DatabaseAdapterSpec's cleanup() (a SELECT bounded by the `maxrows` option,
// then a DELETE ... WHERE id IN (:ids) using a list=true param). The cluster's
// "DELETE ... LIMIT" hypothesis was a red herring — these two query primitives
// are the actual gaps.
//   (A) the `maxrows` queryExecute option must cap the result set.
//   (B) a list=true query param must expand into a parenthesized value list.
// Lucee/ACF honor both; RustCFML ignores maxrows (returns all rows) and does not
// expand list=true (matches 0 rows).
// ============================================================

q = queryNew("id", "integer", [[1],[2],[3],[4],[5]]);

// (A) maxrows
capped = queryExecute("SELECT id FROM q ORDER BY id", {}, {dbtype = "query", maxrows = 2});
assert("the maxrows option caps the result set to 2 rows", capped.recordCount, 2);

// (B) list:true expansion into IN()
inList = queryExecute("SELECT id FROM q WHERE id IN (:ids)", {ids = {value = "1,2,3", list = true}}, {dbtype = "query"});
assert("a list=true param expands into an IN() value list (3 matches)", inList.recordCount, 3);

suiteEnd();
</cfscript>
