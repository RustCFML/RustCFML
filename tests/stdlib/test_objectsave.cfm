<cfscript>
suiteBegin("objectSave / objectLoad");

// objectSave returns binary; objectLoad round-trips it. RustCFML uses an
// internal (non-JVM) format that is only guaranteed to round-trip with itself,
// which is exactly how ColdBox's cache DiskStore marshaller uses the pair.

// --- scalars ---
saved = objectSave( "hello world" );
assertTrue( "objectSave returns binary", isBinary( saved ) );
assert( "string round-trip", objectLoad( saved ), "hello world" );

assert( "int round-trip", objectLoad( objectSave( 42 ) ), 42 );
assert( "double round-trip", objectLoad( objectSave( 3.14 ) ), 3.14 );
assertTrue( "boolean round-trip", objectLoad( objectSave( true ) ) );

// --- struct ---
s = { name="Alex", age=40, active=true, nested={ a=1, b=[1,2,3] } };
r = objectLoad( objectSave( s ) );
assert( "struct.name", r.name, "Alex" );
assert( "struct.age", r.age, 40 );
assert( "struct.nested.a", r.nested.a, 1 );
assert( "struct.nested.b[2]", r.nested.b[2], 2 );

// --- array ---
a = [ "x", "y", 3, { k="v" } ];
ra = objectLoad( objectSave( a ) );
assert( "array len", arrayLen( ra ), 4 );
assert( "array[1]", ra[1], "x" );
assert( "array[4].k", ra[4].k, "v" );

// --- query ---
q = queryNew( "id,title", "integer,varchar" );
queryAddRow( q );
querySetCell( q, "id", 1 );
querySetCell( q, "title", "First" );
queryAddRow( q );
querySetCell( q, "id", 2 );
querySetCell( q, "title", "Second" );
rq = objectLoad( objectSave( q ) );
assertTrue( "query round-trip is query", isQuery( rq ) );
assert( "query recordcount", rq.recordCount, 2 );
assert( "query row2 title", rq.title[2], "Second" );

// --- ColdBox marshaller pattern: toBase64(objectSave(x)) then objectLoad(toBinary(...)) ---
payload = { greeting="hi", items=[10,20,30] };
b64 = toBase64( objectSave( payload ) );
assertTrue( "base64 is a string", isSimpleValue( b64 ) );
back = objectLoad( toBinary( b64 ) );
assert( "marshaller pattern greeting", back.greeting, "hi" );
assert( "marshaller pattern items[3]", back.items[3], 30 );

// --- error on non-objectSave binary input ---
assertThrows( "objectLoad rejects foreign binary", function() {
	objectLoad( toBinary( toBase64( "not a saved object" ) ) );
} );

suiteEnd();
</cfscript>
