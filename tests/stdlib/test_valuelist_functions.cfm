<cfscript>
suiteBegin("valueList / quotedValueList");

// Create a test query
q = queryNew("name,age", "varchar,integer");
queryAddRow(q);
querySetCell(q, "name", "Alice", 1);
querySetCell(q, "age", 30, 1);
queryAddRow(q);
querySetCell(q, "name", "Bob", 2);
querySetCell(q, "age", 25, 2);
queryAddRow(q);
querySetCell(q, "name", "Charlie", 3);
querySetCell(q, "age", 35, 3);

// valueList basic (query.column resolves to an array via dot notation)
assert("valueList basic", valueList(q.name), "Alice,Bob,Charlie");

// valueList with custom delimiter
assert("valueList custom delimiter", valueList(q.name, "|"), "Alice|Bob|Charlie");

// valueList with numeric column
assert("valueList numeric", valueList(q.age), "30,25,35");

// quotedValueList basic
assert("quotedValueList basic", quotedValueList(q.name), "'Alice','Bob','Charlie'");

// quotedValueList with custom delimiter
assert("quotedValueList custom delimiter", quotedValueList(q.name, "|"), "'Alice'|'Bob'|'Charlie'");

// Empty query
emptyQ = queryNew("name", "varchar");
// On an empty query Lucee degrades the column to an Array, which valueList
// refuses; here it stays a column and yields "". Guarded so the valueArray
// suite below still runs cross-engine.
if ( isRustCFML() ) {
    assert("valueList empty query", valueList(emptyQ.name), "");
}

suiteEnd();

// ---------------------------------------------------------------------------
suiteBegin("valueArray");

// valueArray(query, columnName) — array of that column's values (Lucee form;
// used by Preside MultiSelectPanel/queryUtils/ScheduledExportService).
va = valueArray(q, "name");
assertTrue("valueArray(q,col) is an array", isArray(va));
assert("valueArray(q,col) length", arrayLen(va), 3);
assert("valueArray(q,col) joined", arrayToList(va), "Alice,Bob,Charlie");

// valueArray(query.column) — the dot-access already yields a column.
assert("valueArray(q.column) joined", arrayToList(valueArray(q.age)), "30,25,35");

// valueArray of a plain array returns its values — a RustCFML superset;
// Lucee's valueArray takes a query column only.
if ( isRustCFML() ) {
    assert("valueArray(array)", arrayToList(valueArray([1,2,3])), "1,2,3");
}

// Empty query column.
assert("valueArray empty query", arrayLen(valueArray(emptyQ, "name")), 0);

suiteEnd();
</cfscript>
