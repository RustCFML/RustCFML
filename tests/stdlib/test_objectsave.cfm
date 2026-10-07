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

// --- Preside full-page cache shape: HTML full of escapes, nested rc/prc ---
// objectLoad reads this with a direct reader (falling back to serde_json); the
// round trip must be exact.
html = '<p class="a">Line one' & chr(10) & chr(9) & 'tab "quoted" \back\slash /path</p>' & chr(13) & chr(10) & 'café € ' & chr(128512);
page = {
	body = html,
	contentType = "",
	data = {
		rc  = { body = html & html, event = "page.index" },
		prc = { presidePage = { id = "ABC", title = "Home", sort_order = 3, active = true, tags = [ "x", "y" ] }, n = -42, big = 9007199254740993 }
	}
};
q = queryNew( "id,name", "integer,varchar", [ [ 1, "one" ], [ 2, 'two "2"' ] ] );
page.data.prc.q = q;
page.data.prc.when = createDateTime( 2026, 10, 6, 21, 30, 15 );
back = objectLoad( toBinary( toBase64( objectSave( page ) ) ) );
assert( "page body round-trips exactly", back.body, html );
assert( "page body length", len( back.body ), len( html ) );
assert( "rc.body round-trips", back.data.rc.body, html & html );
assert( "nested struct value", back.data.prc.presidePage.title, "Home" );
assert( "integer stays an integer", back.data.prc.presidePage.sort_order, 3 );
assertTrue( "boolean stays boolean", back.data.prc.presidePage.active );
assert( "array inside", back.data.prc.presidePage.tags[ 2 ], "y" );
assert( "negative integer", back.data.prc.n, -42 );
assert( "large integer", back.data.prc.big, 9007199254740993 );
assert( "query rows", back.data.prc.q.recordCount, 2 );
assert( "query value with quotes", back.data.prc.q.name[ 2 ], 'two "2"' );
assert( "datetime round-trips", dateTimeFormat( back.data.prc.when, "yyyy-mm-dd HH:nn:ss" ), "2026-10-06 21:30:15" );

// --- Lucee's file forms: objectSave( value, path ) writes the bytes; objectLoad( path ) reads them ---
fpath = getTempDirectory() & "objectsave_" & createUUID() & ".bin";
saved = objectSave( { a = 1, s = "file form" }, fpath );
assertTrue( "objectSave with a path still returns the binary", isBinary( saved ) );
assertTrue( "objectSave with a path writes the file", fileExists( fpath ) );
assert( "the file holds exactly the returned bytes", arrayLen( fileReadBinary( fpath ) ), arrayLen( saved ) );
fromFile = objectLoad( fpath );
assert( "objectLoad( path ) reads the file", fromFile.s, "file form" );
fileDelete( fpath );

// --- type fidelity ---
assert( "a fractional double round-trips", objectLoad( objectSave( 1.5 ) ), 1.5 );
tagged = objectLoad( objectSave( { _cftype = "binary", data = "zz", other = 1 } ) );
assertTrue( "a user struct with a _cftype key stays a struct", isStruct( tagged ) );
assert( "and keeps its keys", tagged.data, "zz" );

if ( isRustCFML() ) {
	// Blobs written before format 2 (JSON inside the RCFMLOBJ header) must still
	// load: existing cache files on disk are in that format.
	v1 = objectLoad( toBinary( "UkNGTUxPQkoBeyJncmVldGluZyI6ImhpIiwiaHRtbCI6IjxwPmFcImJcIjwvcD5cblx0eCIsIm4iOjQyLCJpdGVtcyI6WzEsInR3byJdfQ==" ) );
	assert( "format-1 blob: string", v1.greeting, "hi" );
	assert( "format-1 blob: escaped html", v1.html, '<p>a"b"</p>' & chr(10) & chr(9) & "x" );
	assert( "format-1 blob: number", v1.n, 42 );
	assert( "format-1 blob: array", v1.items[ 2 ], "two" );
	hex = binaryEncode( objectSave( { k = repeatString( "x", 100 ) } ), "hex" );
	truncated = binaryDecode( left( hex, len( hex ) - 20 ), "hex" );
	msg = "";
	try { objectLoad( truncated ); } catch ( any e ) { msg = e.message; }
	assertTrue( "a truncated blob is an objectLoad error", findNoCase( "objectLoad", msg ) > 0 );
}

// --- error on non-objectSave binary input ---
assertThrows( "objectLoad rejects foreign binary", function() {
	objectLoad( toBinary( toBase64( "not a saved object" ) ) );
} );

suiteEnd();
</cfscript>
