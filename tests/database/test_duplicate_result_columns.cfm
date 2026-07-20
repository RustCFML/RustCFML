<cfscript>
suiteBegin("Duplicate column names in a SQL result set (GH-279)");

// Lucee collapses a SQL result set's duplicate column names to the FIRST
// occurrence: the later same-named column is discarded entirely, name AND data.
// RustCFML previously kept both entries (columnList "ID,ID") and every read
// resolved to the LAST column's value. Exercised through a real SQLite result
// set — `queryNew("id,id",...)` is NOT a faithful repro (Lucee rejects it).
memDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };

q = queryExecute( "select 'F362CE46-UUID' as id, 1 as id", {}, { datasource = memDs } );

assert( "duplicate columns collapse to a single entry in columnList", q.columnList, "ID" );
assert( "unqualified dot read returns the FIRST column's value", q.id, "F362CE46-UUID" );
assert( "bracket read returns the FIRST column's value", q[ "id" ][ 1 ], "F362CE46-UUID" );
assert( "recordCount is unaffected", q.recordCount, 1 );

// Case-insensitive collapse: `id` and `ID` are the same column.
q2 = queryExecute( "select 'first' as id, 'second' as ID", {}, { datasource = memDs } );
assert( "case-insensitive duplicate also collapses", q2.columnList, "ID" );
assert( "case-insensitive collapse keeps the first value", q2.id, "first" );

// A genuinely distinct trailing column after a duplicate survives, in order.
q3 = queryExecute( "select 'a' as id, 2 as id, 'keep' as name", {}, { datasource = memDs } );
assert( "distinct later column survives the dedup", q3.columnList, "ID,NAME" );
assert( "first duplicate value wins", q3.id, "a" );
assert( "distinct column reads correctly", q3.name, "keep" );

suiteEnd();
</cfscript>
