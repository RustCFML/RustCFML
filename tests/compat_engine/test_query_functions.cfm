<cfscript>
// Lucee 7 Compatibility Tests: Query Functions
// Synthesized from https://github.com/lucee/Lucee/tree/7.0/test/functions
// Original tests Copyright (c) 2014, the Railo Company LLC / Copyright (c) 2015-2016, Lucee Association Switzerland
// Licensed under the GNU Lesser General Public License v2.1
// Adapted for RustCFML test harness

// Helper: build a single-column query with given values
function buildQuery(colName, colType, values) {
    var q = queryNew(colName, colType);
    for (var v in values) {
        queryAddRow(q);
        querySetCell(q, colName, v, q.recordcount);
    }
    return q;
}

// ============================================================
// QueryNew (from Lucee QueryNew.cfc)
// ============================================================
suiteBegin("Lucee7: QueryNew");
q1 = queryNew("name,age", "varchar,integer");
assert("queryNew empty recordcount", q1.recordcount, 0);
assertTrue("queryNew columnlist contains name", findNoCase("name", q1.columnlist) > 0);
assertTrue("queryNew columnlist contains age", findNoCase("age", q1.columnlist) > 0);

// queryNew with data - build manually
q2 = queryNew("name,age", "varchar,integer");
queryAddRow(q2);
querySetCell(q2, "name", "Susi", 1);
querySetCell(q2, "age", 25, 1);
queryAddRow(q2);
querySetCell(q2, "name", "Urs", 2);
querySetCell(q2, "age", 30, 2);
assert("queryNew with data recordcount", q2.recordcount, 2);
assert("queryNew with data name[1]", q2.name[1], "Susi");
assert("queryNew with data name[2]", q2.name[2], "Urs");
assert("queryNew with data age[1]", q2.age[1], 25);
assert("queryNew with data age[2]", q2.age[2], 30);
suiteEnd();

// ============================================================
// QueryAddRow (from Lucee QueryAddRow.cfc)
// ============================================================
suiteBegin("Lucee7: QueryAddRow");
q = queryNew("name", "varchar");
queryAddRow(q);
assert("queryAddRow increases count", q.recordcount, 1);
queryAddRow(q);
assert("queryAddRow second row", q.recordcount, 2);
queryAddRow(q, 3);
assert("queryAddRow multiple rows", q.recordcount, 5);
suiteEnd();

// ============================================================
// QueryAddColumn (from Lucee QueryAddColumn.cfc)
// ============================================================
suiteBegin("Lucee7: QueryAddColumn");
q = queryNew("name");
queryAddColumn(q, "age", "integer", []);
assertTrue("queryAddColumn column exists", findNoCase("age", q.columnlist) > 0);
suiteEnd();

// ============================================================
// QuerySetCell / QueryGetCell (from Lucee QuerySetCell.cfc)
// ============================================================
suiteBegin("Lucee7: QuerySetCell/QueryGetCell");
q = queryNew("name", "varchar");
queryAddRow(q);
querySetCell(q, "name", "test", 1);
assert("querySetCell initial value", q.name[1], "test");
querySetCell(q, "name", "updated", 1);
assert("querySetCell updates value", q.name[1], "updated");
assert("queryGetCell reads value", queryGetCell(q, "name", 1), "updated");
suiteEnd();

// ============================================================
// QueryColumnArray / QueryColumnCount (from Lucee QueryColumnArray.cfc, QueryColumnCount.cfc)
// ============================================================
suiteBegin("Lucee7: QueryColumnArray/QueryColumnCount");
q = queryNew("a,b,c");
assert("queryColumnCount", queryColumnCount(q), 3);
// queryColumnArray returns column NAMES. A one-column query cannot tell the
// names apart from the values — arrayLen is 1 either way — so this pinned GH
// #345 (the function was aliased to queryColumnData and returned an empty
// array) rather than catching it. Assert the contents, on a query whose column
// names and cell values differ.
q2 = buildQuery("x", "varchar", ["test"]);
cols = queryColumnArray(q2);
assertTrue("queryColumnArray is array", isArray(cols));
assert("queryColumnArray length", arrayLen(cols), 1);
assert("queryColumnArray holds the name, not the value", cols[1], "x");
q2b = queryNew("ColA,fullName", "integer,varchar");
assert("queryColumnArray lists every column, casing intact", arrayToList(queryColumnArray(q2b)), "ColA,fullName");
suiteEnd();

// ============================================================
// QueryColumnExists (from Lucee QueryColumnExists.cfc)
// ============================================================
suiteBegin("Lucee7: QueryColumnExists");
q = queryNew("name,age");
assertTrue("queryColumnExists found", queryColumnExists(q, "name"));
assertTrue("queryColumnExists case insensitive", queryColumnExists(q, "NAME"));
assertFalse("queryColumnExists missing", queryColumnExists(q, "missing"));
suiteEnd();

// ============================================================
// QueryRecordCount (from Lucee QueryRecordCount.cfc)
// ============================================================
suiteBegin("Lucee7: QueryRecordCount");
q = buildQuery("x", "varchar", ["a", "b", "c"]);
assert("queryRecordCount", queryRecordCount(q), 3);
assert("queryRecordCount via property", q.recordcount, 3);
suiteEnd();

// ============================================================
// QueryDeleteRow (from Lucee QueryDeleteRow.cfc)
// ============================================================
suiteBegin("Lucee7: QueryDeleteRow");
q = buildQuery("x", "varchar", ["a", "b", "c"]);
queryDeleteRow(q, 2);
assert("queryDeleteRow count", q.recordcount, 2);
assert("queryDeleteRow first still a", q.x[1], "a");
assert("queryDeleteRow second now c", q.x[2], "c");
suiteEnd();

// ============================================================
// QueryDeleteColumn (from Lucee QueryDeleteColumn.cfc)
// ============================================================
suiteBegin("Lucee7: QueryDeleteColumn");
q = queryNew("a,b", "varchar,varchar");
queryAddRow(q);
querySetCell(q, "a", "1", 1);
querySetCell(q, "b", "2", 1);
queryDeleteColumn(q, "b");
assert("queryDeleteColumn count", queryColumnCount(q), 1);
assertFalse("queryDeleteColumn removed", queryColumnExists(q, "b"));
suiteEnd();

// ============================================================
// QuerySlice (from Lucee QuerySlice.cfc)
// ============================================================
suiteBegin("Lucee7: QuerySlice");
q = buildQuery("x", "varchar", ["a", "b", "c", "d"]);
q2 = querySlice(q, 2, 2);
assert("querySlice recordcount", q2.recordcount, 2);
assert("querySlice first row", q2.x[1], "b");
assert("querySlice second row", q2.x[2], "c");
suiteEnd();

// ============================================================
// QueryEach (from Lucee QueryEach.cfc)
// ============================================================
suiteBegin("Lucee7: QueryEach");
q = buildQuery("x", "integer", [1, 2, 3]);
total = 0;
queryEach(q, function(row){ total = total + row.x; });
assert("queryEach sum", total, 6);
suiteEnd();

// ============================================================
// QueryFilter (from Lucee QueryFilter.cfc)
// ============================================================
suiteBegin("Lucee7: QueryFilter");
q = buildQuery("x", "integer", [1, 2, 3]);
q2 = queryFilter(q, function(row){ return row.x > 1; });
assert("queryFilter recordcount", q2.recordcount, 2);
suiteEnd();

// ============================================================
// QueryMap (from Lucee QueryMap.cfc)
// ============================================================
suiteBegin("Lucee7: QueryMap");
q = buildQuery("x", "integer", [1, 2, 3]);
q2 = queryMap(q, function(row){ row.x = row.x * 2; return row; });
assert("queryMap first doubled", q2.x[1], 2);
assert("queryMap second doubled", q2.x[2], 4);
assert("queryMap third doubled", q2.x[3], 6);
suiteEnd();

// ============================================================
// QuerySort (from Lucee QuerySort.cfc)
// ============================================================
suiteBegin("Lucee7: QuerySort");
q = buildQuery("x", "integer", [3, 1, 2]);
querySort(q, function(a, b){ return a.x - b.x; });
assert("querySort first", q.x[1], 1);
assert("querySort second", q.x[2], 2);
assert("querySort third", q.x[3], 3);
suiteEnd();

// ============================================================
// QuerySome / QueryEvery (from Lucee QuerySome.cfc, QueryEvery.cfc)
// ============================================================
suiteBegin("Lucee7: QuerySome/QueryEvery");
q = buildQuery("x", "integer", [1, 2, 3]);
assertTrue("querySome finds match", querySome(q, function(row){ return row.x > 2; }));
assertFalse("querySome no match", querySome(q, function(row){ return row.x > 5; }));

q2 = buildQuery("x", "integer", [2, 4, 6]);
assertTrue("queryEvery all even", queryEvery(q2, function(row){ return row.x mod 2 == 0; }));
assertFalse("queryEvery not all even", queryEvery(q, function(row){ return row.x mod 2 == 0; }));
suiteEnd();

// ============================================================
// QueryReduce (from Lucee QueryReduce.cfc)
// ============================================================
suiteBegin("Lucee7: QueryReduce");
q = buildQuery("x", "integer", [1, 2, 3]);
sum = queryReduce(q, function(acc, row){ return acc + row.x; }, 0);
assert("queryReduce sum", sum, 6);
suiteEnd();

// ============================================================
// IsQuery (from Lucee IsQuery.cfc)
// ============================================================
suiteBegin("Lucee7: IsQuery");
assertTrue("isQuery with query", isQuery(queryNew("x")));
assertFalse("isQuery with struct", isQuery({}));
assertFalse("isQuery with string", isQuery("hello"));
assertFalse("isQuery with number", isQuery(42));
assertFalse("isQuery with array", isQuery([]));
suiteEnd();
</cfscript>
