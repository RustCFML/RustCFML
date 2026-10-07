<cfscript>
suiteBegin("serializeJSON query formats, toBinary and query equality (GH 469, 472, 473)");

// GH 469 — the second argument also accepts "row", "column" and "struct".
q = queryNew( "id,name", "integer,varchar", [ [ 1, "a" ], [ 2, "b" ] ] );
rowForm = '{"COLUMNS":["id","name"],"DATA":[[1,"a"],[2,"b"]]}';
colForm = '{"ROWCOUNT":2,"COLUMNS":["id","name"],"DATA":{"ID":[1,2],"NAME":["a","b"]}}';
assert("default is row format", serializeJson( q ), rowForm);
assert("false is row format", serializeJson( q, false ), rowForm);
assert("true is column format", serializeJson( q, true ), colForm);
assert('"column" is column format', serializeJson( q, "column" ), colForm);
assert('"row" is row format', serializeJson( q, "row" ), rowForm);
assert('"struct" is an array of row objects',
	serializeJson( q, "struct" ), '[{"id":1,"name":"a"},{"id":2,"name":"b"}]');

// GH 472 — characters outside the base64 alphabet are SKIPPED, not decoded as
// zero bits (which fabricated three leading bytes here).
b = toBinary( "!!!notbase64" );
assert("invalid characters are skipped", binaryEncode( b, "hex" ), "9E8B5B6AC7BA");
assert("unpadded base64 still decodes", toString( toBinary( "aGVsbG8" ) ), "hello");

// GH 473 — a query is compared by its columns and cells, not by its (identical
// for every query) string form, which made the first query in an array match
// any query at all and broke cycle guards built on indexOf.
q1 = queryNew( "id,label", "varchar,varchar", [ [ "a", "A" ] ] );
q2 = queryNew( "id,label", "varchar,varchar", [ [ "b", "B" ] ] );
q3 = queryNew( "id,label", "varchar,varchar", [ [ "a", "A" ] ] );
q4 = queryNew( "x", "varchar", [ [ "z" ] ] );
arr = [ q1 ];
assert("indexOf: different data does not match", arr.indexOf( q2 ), -1);
assert("indexOf: different columns do not match", arr.indexOf( q4 ), -1);
assert("indexOf: the same query matches", arr.indexOf( q1 ), 0);
assert("indexOf: an equal query matches", arr.indexOf( q3 ), 0);
assert("arrayFind: different data does not match", arrayFind( arr, q2 ), 0);
assert("arrayFind: an equal query matches", arrayFind( arr, q3 ), 1);
assert("arrayContains: different data does not match", arrayContains( arr, q2 ), 0);
assert("query.equals compares contents", q1.equals( q2 ), false);
assert("query.equals is true for equal queries", q1.equals( q3 ), true);
// Structs and simple values are unchanged.
assert("structs still compare by content", [ { a = 1 } ].indexOf( { a = 2 } ), -1);
assert("strings still compare by value", [ "x", "y" ].indexOf( "y" ), 1);

suiteEnd();
</cfscript>
