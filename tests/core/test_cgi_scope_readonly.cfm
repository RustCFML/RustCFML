<cfscript>
// GitHub #372 — the `cgi` scope is read-only to CFML code. RustCFML used to
// accept every write and read the value back; Lucee refuses them.
//
// The mark rides on the SCOPE STRUCT, not on the name `cgi`, which is what
// Lucee models too: an alias (`local.c = cgi; local.c.x = 1`) is refused just
// the same. `url`/`form`/`cookie` stay writable on both engines.
//
// Which operations throw was taken from Lucee 7.1.0+204, not assumed — Lucee
// rejects the SET forms and `structClear`, but lets `structDelete`/`structAppend`
// through (where it then silently does nothing). We deliberately do NOT copy
// that silent no-op, and equally do not throw where Lucee doesn't; see
// docs/known-issues.md.
suiteBegin("cgi scope is read-only (GH 372)");

// ---------------------------------------------------------------- refused
assertThrows( "dot assignment", function(){ cgi.rotest = "x"; } );
assertThrows( "bracket assignment", function(){ cgi[ "rotest" ] = "x"; } );
assertThrows( "structInsert", function(){ structInsert( cgi, "rotest", "x" ); } );
assertThrows( "structUpdate", function(){ structUpdate( cgi, "rotest", "x" ); } );
assertThrows( "member insert()", function(){ cgi.insert( "rotest", "x" ); } );
assertThrows( "structClear", function(){ structClear( cgi ); } );
// A null RHS deletes rather than stores, but Lucee compiles it as a set and
// rejects it as one.
assertThrows( "null assignment", function(){ cgi.rotest = javacast( "null", "" ); } );

// Refused through an ALIAS as well — the struct is read-only, not the name.
aliased = cgi;
assertThrows( "write through an alias", function(){ aliased.rotest = "x"; } );

// Refused inside a function too (the shape the issue reported).
function writeCgi() {
	cgi.qtest = "x";
	return "WROTE";
}
assertThrows( "write inside a function", writeCgi );

// Nothing landed.
assertFalse( "no key was created", structKeyExists( cgi, "rotest" ) );
assertFalse( "no key was created inside the function", structKeyExists( cgi, "qtest" ) );

// The refusal is CATCHABLE — a page that guards the write must be able to.
caught = "";
try {
	cgi.rotest = "x";
} catch ( any e ) {
	caught = e.message;
}
assert( "refusal is catchable and Lucee-worded", caught, "can't set key [ROTEST] to struct, struct is readonly" );

// Lucee echoes a string-literal key as written and an identifier key
// upper-cased (its compiler upper-cases member names).
caughtLiteral = "";
try {
	cgi[ "roTest" ] = "x";
} catch ( any e ) {
	caughtLiteral = e.message;
}
assert( "literal key keeps its casing", caughtLiteral, "can't set key [roTest] to struct, struct is readonly" );

// ---------------------------------------------------------------- allowed
// Reads are untouched.
assert( "cgi reads still work", isStruct( cgi ), true );

// `cgi` as the SOURCE of a merge is a read, not a write.
merged = {};
structAppend( merged, cgi );
assert( "cgi can be merged FROM", isStruct( merged ), true );

// The other request scopes are writable on both engines — the read-only mark
// must not have leaked onto them.
url.rotest = "u";
form.rotest = "f";
cookie.rotest = "c";
assert( "url stays writable", url.rotest, "u" );
assert( "form stays writable", form.rotest, "f" );
assert( "cookie stays writable", cookie.rotest, "c" );
structDelete( url, "rotest" );
structDelete( form, "rotest" );
structDelete( cookie, "rotest" );

// A plain struct is of course unaffected.
plain = { a = 1 };
plain.b = 2;
assert( "an ordinary struct is still writable", plain.b, 2 );

suiteEnd();
</cfscript>
