<cfscript>
suiteBegin("Query Functions");

// --- queryNew with column list ---
q1 = queryNew("id,name");
assert("queryNew creates 0 rows", q1.recordCount, 0);

// --- queryNew with types ---
q2 = queryNew("id,name,age", "integer,varchar,integer");
assert("queryNew with types has 0 rows", q2.recordCount, 0);

// --- queryAddRow ---
queryAddRow(q2);

// --- querySetCell ---
querySetCell(q2, "id", 1);
querySetCell(q2, "name", "Alice");
querySetCell(q2, "age", 30);

// --- recordCount ---
assert("recordCount after 1 add", q2.recordCount, 1);

// --- add more rows ---
queryAddRow(q2);
querySetCell(q2, "id", 2, 2);
querySetCell(q2, "name", "Bob", 2);
querySetCell(q2, "age", 25, 2);

queryAddRow(q2);
querySetCell(q2, "id", 3, 3);
querySetCell(q2, "name", "Charlie", 3);
querySetCell(q2, "age", 35, 3);

assert("recordCount after 3 adds", q2.recordCount, 3);

// --- columnList ---
cols = q2.columnList;
assertTrue("columnList contains ID", listFindNoCase(cols, "id") > 0);
assertTrue("columnList contains NAME", listFindNoCase(cols, "name") > 0);
assertTrue("columnList contains AGE", listFindNoCase(cols, "age") > 0);

// --- queryGetRow ---
row1 = queryGetRow(q2, 1);
assert("queryGetRow name", row1.name, "Alice");
assert("queryGetRow age", row1.age, 30);

// --- queryColumnExists ---
assertTrue("queryColumnExists name", queryColumnExists(q2, "name"));
assertFalse("queryColumnExists nope", queryColumnExists(q2, "nope"));

// --- queryDeleteRow ---
q3 = queryNew("id,name", "integer,varchar");
queryAddRow(q3);
querySetCell(q3, "id", 1);
querySetCell(q3, "name", "X");
queryAddRow(q3);
querySetCell(q3, "id", 2, 2);
querySetCell(q3, "name", "Y", 2);
queryDeleteRow(q3, 1);
assert("queryDeleteRow reduces count", q3.recordCount, 1);

// --- queryAddColumn ---
q4 = queryNew("id", "integer");
queryAddRow(q4);
querySetCell(q4, "id", 1);
queryAddColumn(q4, "email", "varchar", ["test@example.com"]);
assertTrue("queryAddColumn adds column", queryColumnExists(q4, "email"));

// --- querySlice ---
sliced = querySlice(q2, 1, 2);
assert("querySlice returns 2 rows", sliced.recordCount, 2);

// --- queryColumnData ---
ages = queryColumnData(q2, "age");
assert("queryColumnData returns array", arrayLen(ages), 3);
assert("queryColumnData first value", ages[1], 30);

// --- SQL execution errors are catchable `database`-typed exceptions ---
// Lucee/ACF surface SQL failures as type="database", and CFML code routinely
// does `catch( database e )` (e.g. Preside's cascade-delete guard relies on a
// FK/constraint violation arriving as a database exception). A bad query must
// NOT be a generic runtime error.
dbErrDs = "sqlite://" & getTempDirectory() & "/rustcfml_dberr_" & createUUID() & ".sqlite";
try { queryExecute("CREATE TABLE t_dberr (id INTEGER PRIMARY KEY)", [], {datasource: dbErrDs}); } catch (any e) {}
caughtAsDatabase = false;
try {
    queryExecute("SELECT * FROM no_such_table_here", [], {datasource: dbErrDs});
} catch (database e) {
    caughtAsDatabase = true;
} catch (any e) {
    caughtAsDatabase = false;
}
assertTrue("SQL error is catchable as type='database'", caughtAsDatabase);

// --- queryNew() with a flat scalar array chunks by column count into rows ---
// Lucee: queryNew("width,height","int,int",[100,200]) is ONE row (not two
// single-value rows). Preside's AssetManagerService.getAssetDimensions relies
// on this — its mocks build dimension queries this way and read q.width/q.height
// as scalars. A flat array whose length is a multiple of the column count is
// chunked into rows of columns.len() values.
qOneRow = queryNew("width,height", "int,int", [100, 200]);
assert("flat array -> one row", qOneRow.recordCount, 1);
assert("flat array row width scalar", qOneRow.width, 100);
assert("flat array row height scalar", qOneRow.height, 200);
assertTrue("flat array width isNumeric", isNumeric(qOneRow.width));
// Chunking a FLAT array into rows-of-columns is a RustCFML behaviour: Lucee
// makes a single row from [1,2,3,4] regardless of column count. The one-row
// case above agrees on both engines; only the multi-row chunk diverges.
qTwoRows = queryNew("a,b", "int,int", [1, 2, 3, 4]);
if ( isRustCFML() ) {
    assert("flat array -> two rows", qTwoRows.recordCount, 2);
    assert("row1 a", queryGetRow(qTwoRows, 1).a, 1);
    assert("row2 b", queryGetRow(qTwoRows, 2).b, 4);
}
// Single-column shortcut yields one row per scalar — same RustCFML flat-array
// handling as above; Lucee makes a single row from any flat array.
qSingleCol = queryNew("id", "int", [1, 2, 3]);
if ( isRustCFML() ) {
    assert("single-column flat array -> row per value", qSingleCol.recordCount, 3);
}
// Array-of-arrays (explicit rows) and array-of-structs unaffected.
qRows = queryNew("a,b", "int,int", [[10, 20], [30, 40]]);
assert("array-of-arrays -> two rows", qRows.recordCount, 2);
assert("array-of-arrays row2 a", queryGetRow(qRows, 2).a, 30);

// --- GH #344: duplicate column names are refused (both entry points) ---
// A query with two same-named columns has no well-defined semantics for
// `q.ColA`, valueList, serialisation or QoQ, so Lucee refuses to build one and
// so do we. Column names are case-insensitive, so the differing-case spelling
// is the SAME column and must throw as well. Lucee surfaces both as
// type="database", verified on 7.1.0.204.
assertThrows("queryAddColumn rejects an exact duplicate", function() {
    var d = queryNew("ColA", "integer", [[1]]);
    queryAddColumn(d, "ColA", [2]);
});
assertThrows("queryAddColumn rejects a differing-case duplicate", function() {
    var d = queryNew("ColA", "integer", [[1]]);
    queryAddColumn(d, "COLA", [2]);
});
assertThrows("queryNew rejects a duplicate in the column list", function() {
    queryNew("ColA,COLA", "integer,integer");
});
assertThrows("queryNew rejects a duplicate in a column array", function() {
    queryNew(["ColA", "COLA"]);
});
dupErrType = "";
try {
    dupQ = queryNew("ColA", "integer", [[1]]);
    queryAddColumn(dupQ, "cola", [2]);
} catch (any e) {
    dupErrType = e.type;
}
assert("duplicate column error is type=database", dupErrType, "database");
// A genuinely new column is of course still fine.
okQ = queryNew("ColA", "integer", [[1]]);
queryAddColumn(okQ, "ColB", [2]);
assert("distinct column still added", okQ.columnList, "COLA,COLB");

// --- queryAddColumn's optional datatype argument ---
// Lucee's signature is queryAddColumn(query, name, [datatype], array): the type
// sits BEFORE the values. Reading the array from the third position only meant
// the four-argument spelling silently added an all-null column.
typedQ = queryNew("ColA", "integer", [[1]]);
queryAddColumn(typedQ, "ColB", "integer", [2]);
assert("4-arg queryAddColumn keeps the values", queryGetRow(typedQ, 1).ColB, 2);

// --- addColumn is available as a query member function ---
memberQ = queryNew("ColA", "integer", [[1]]);
memberQ.addColumn("ColB", [2]);
assert("member addColumn adds the column", memberQ.columnList, "COLA,COLB");

// --- GH #345: querySort takes a COLUMN NAME, not just a comparator ---
// Lucee's common spelling is querySort(q, "col" [, "asc|desc"]); the closure
// form is the rarer one. Both sort the shared query handle IN PLACE and the
// FUNCTION returns a boolean — verified on Lucee 7.1.0.204. Storing the return
// value back over the query would therefore clobber it, which is why querySort
// is NOT in codegen's is_mutating_standalone_call list.
sortQ = queryNew("ColA,fullName", "integer,varchar", [[2, "b"], [1, "a"], [3, "c"]]);
sortRet = querySort(sortQ, "ColA");
assert("querySort by column sorts ascending", valueList(sortQ.ColA), "1,2,3");
assert("querySort returns a boolean", sortRet, true);
querySort(sortQ, "ColA", "desc");
assert("querySort desc", valueList(sortQ.ColA), "3,2,1");
querySort(sortQ, "cola", "DESC");
assert("querySort column and direction are case-insensitive", valueList(sortQ.ColA), "3,2,1");
querySort(sortQ, "ColA", "");
assert("querySort blank direction is ascending", valueList(sortQ.ColA), "1,2,3");
// The direction list is PARALLEL to the column list — one entry per column, not
// a suffix on each name. Lucee reads "b desc" as a column literally called
// "b desc" and says so.
twoColQ = queryNew("a,b", "integer,integer", [[1, 2], [1, 1], [0, 9]]);
querySort(twoColQ, "a,b", "asc,desc");
assert("querySort multi-column a", valueList(twoColQ.a), "0,1,1");
assert("querySort multi-column b tie-break", valueList(twoColQ.b), "9,2,1");
assertThrows("querySort rejects a direction count mismatch", function() {
    var d = queryNew("a,b", "integer,integer", [[1, 2]]);
    querySort(d, "a,b", "desc");
});
assertThrows("querySort rejects an unknown column", function() {
    var d = queryNew("a", "integer", [[1]]);
    querySort(d, "nope");
});
assertThrows("querySort rejects a bad direction", function() {
    var d = queryNew("a", "integer", [[1]]);
    querySort(d, "a", "sideways");
});
sortErrType = "";
try {
    badSortQ = queryNew("a", "integer", [[1]]);
    querySort(badSortQ, "nope");
} catch (any e) {
    sortErrType = e.type;
}
assert("querySort errors are type=database", sortErrType, "database");
// Sorting is stable, and empty/null cells sort first ascending (a plain
// reversal puts them last descending — not a nulls-last rule).
stableQ = queryNew("a,b", "integer,varchar", [[1, "x"], [1, "y"], [0, "z"]]);
querySort(stableQ, "a");
assert("querySort is stable", valueList(stableQ.b), "z,x,y");
nullQ = queryNew("a", "integer", [[2], [""], [1]]);
querySort(nullQ, "a");
assert("querySort puts empties first ascending", valueList(nullQ.a), ",1,2");
querySort(nullQ, "a", "desc");
assert("querySort puts empties last descending", valueList(nullQ.a), "2,1,");
// An all-numeric column sorts numerically; one non-numeric value makes the
// whole column sort as text. The decision is per COLUMN, not per comparison.
numQ = queryNew("a", "", [["10"], ["9"], ["100"]]);
querySort(numQ, "a");
assert("all-numeric column sorts numerically", valueList(numQ.a), "9,10,100");
mixedQ = queryNew("a", "", [["10"], ["b"], ["2"]]);
querySort(mixedQ, "a");
assert("a non-numeric value makes the column sort as text", valueList(mixedQ.a), "10,2,b");
// Text order is case-SENSITIVE on Lucee: "B" sorts before "a".
caseQ = queryNew("s", "varchar", [["B"], ["a"]]);
querySort(caseQ, "s");
assert("querySort text order is case-sensitive", valueList(caseQ.s), "B,a");
// The member form takes a column too — and unlike the function it returns the
// QUERY, so it can be chained (Lucee does the same).
memberSortQ = queryNew("a", "integer", [[2], [1]]);
memberSortRet = memberSortQ.sort("a", "desc");
assert("member sort by column", valueList(memberSortQ.a), "2,1");
assertTrue("member sort returns the query", isQuery(memberSortRet));
// The comparator form still works, and also returns a boolean.
cbQ = queryNew("a", "integer", [[2], [1], [3]]);
cbRet = querySort(cbQ, function(x, y) { return compare(x.a, y.a); });
assert("querySort comparator form still sorts", valueList(cbQ.a), "1,2,3");
assert("querySort comparator form returns a boolean", cbRet, true);
emptySortQ = queryNew("a", "integer");
assert("querySort on an empty query is a no-op", querySort(emptySortQ, "a"), true);

// --- GH #345: queryColumnArray returns the column NAMES ---
// It was registered as a plain alias of queryColumnData, so the documented
// 1-argument form read a column named "" and handed back an empty array.
namesQ = queryNew("ColA,fullName", "integer,varchar");
assert("queryColumnArray returns the column names", arrayToList(queryColumnArray(namesQ)), "ColA,fullName");
assert("member columnArray returns the column names", arrayToList(namesQ.columnArray()), "ColA,fullName");

// --- GH #345: queryColumnList preserves casing; the PROPERTY uppercases ---
// Lucee draws this line deliberately: `q.columnList` is the legacy cfquery
// pseudo-column (upper on every engine), `queryColumnList(q)` and the
// `q.columnList()` member CALL are the modern accessors and keep the casing.
// Do not "fix" the property to match the function.
assert("queryColumnList preserves casing", queryColumnList(namesQ), "ColA,fullName");
assert("queryColumnList takes a delimiter", queryColumnList(namesQ, "|"), "ColA|fullName");
assert("member columnList() preserves casing", namesQ.columnList(), "ColA,fullName");
assert("member columnList() takes a delimiter", namesQ.columnList("|"), "ColA|fullName");
assert("the columnList PROPERTY still uppercases", namesQ.columnList, "COLA,FULLNAME");
assert("getColumnList(false) preserves casing", namesQ.getColumnList(false), "ColA,fullName");
assert("getColumnList(true) uppercases", namesQ.getColumnList(true), "COLA,FULLNAME");

suiteEnd();
</cfscript>
