<cfscript>
suiteBegin("Struct literal keys: yes/no spelling + null-value enumeration (Lucee parity)");

// ---------------------------------------------------------------------------
// `yes`/`no` are NOT boolean literals in CFML script — Lucee/ACF treat a bare
// `yes`/`no` as an ordinary identifier (`x = yes` throws "variable [YES]
// doesn't exist"). So as unquoted struct-literal KEYS they stay "yes"/"no",
// NOT "true"/"false". Only `true`/`false` are boolean literals. (cfflow's
// WorkflowStateSubstitutionProvider builds `{ yes=…, no=… }` state.)
// ---------------------------------------------------------------------------
s = { yes = 1, no = 2, true = 3, foo = 4 };
keys = listSort( structKeyList( s ), "textnocase" );
assert( "yes/no/true preserved as literal keys", keys, "foo,no,true,yes" );
assert( "value under 'yes' key", s.yes, 1 );
assert( "value under 'no' key",  s.no,  2 );
// true/false still work as boolean literals in expressions
assertTrue(  "true is still a boolean literal", true );
assertFalse( "false is still a boolean literal", false );

// ---------------------------------------------------------------------------
// A struct LITERAL null value creates an ENUMERATED key: for-in, structKeyList
// and structCount include it (verified on Lucee 7.0.4 — `{a=1,x=nullValue(),
// b=2}` → 3 keys), even though structKeyExists reports it absent ("a NULL value
// is the same as not existing"). Reading it is defensive (`?:`).
// ---------------------------------------------------------------------------
n = { a = 1, x = nullValue(), b = 2 };
assertFalse( "structKeyExists is false for null-valued key", structKeyExists( n, "x" ) );
assert( "structCount includes null-valued key", structCount( n ), 3 );
nkeys = structKeyList( n );
assertTrue( "structKeyList includes null-valued key", listFindNoCase( nkeys, "x" ) GT 0 );
seen = "";
for ( var k in n ) { seen = listAppend( seen, k ); }
assertTrue( "for-in enumerates null-valued key", listFindNoCase( seen, "x" ) GT 0 );
assert( "reading the null key via elvis yields default", ( n.x ?: "wasnull" ), "wasnull" );

suiteEnd();
</cfscript>
