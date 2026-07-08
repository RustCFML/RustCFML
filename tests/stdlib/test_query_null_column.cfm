<cfscript>
// A SQL-NULL query cell surfaces as an empty string, NOT as an absent key.
// CFML default (nullSupport=false): every column of a query row is a PRESENT
// key; a NULL cell reads as "". A missing key would break `structKeyExists`,
// `structKeyList`, `for row in q { row.nullCol }`, and — the repro that found
// this — a `param name="args.nullCol" type="string"` over query-row data
// (Preside admin sitetree `_node.cfm`, where the homepage's NULL parent_page
// made the column vanish from the node struct). Lucee/ACF include it as "".
suiteBegin("Query NULL column is empty string, key present");

q = queryNew( "a,b,c", "varchar,varchar,varchar" );
queryAddRow( q );
querySetCell( q, "a", "x" );
querySetCell( q, "c", "z" );
// b left unset -> SQL NULL

for ( row in q ) {
	assertTrue( "columnList lists all columns", ListLen( q.columnList ) == 3 );
	assertTrue( "null column key is present in row struct", structKeyExists( row, "b" ) );
	assertTrue( "null column appears in structKeyList", ListFindNoCase( structKeyList( row ), "b" ) > 0 );
	assert( "null cell reads as empty string", row.b, "" );
	assertFalse( "null cell is not treated as null", isNull( row.b ) );
	assert( "non-null cells intact (a)", row.a, "x" );
	assert( "non-null cells intact (c)", row.c, "z" );
}

// param name=... type="string" over a row's NULL column must NOT throw
function paramNullCol( struct args ) {
	param name="arguments.args.b" type="string";
	return "ok:[" & arguments.args.b & "]";
}
row1 = "";
for ( r in q ) { row1 = r; break; }
assert( "param type=string over a NULL column key succeeds", paramNullCol( row1 ), "ok:[]" );

suiteEnd();
</cfscript>
