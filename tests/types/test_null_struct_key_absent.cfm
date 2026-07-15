<cfscript>
suiteBegin("Null struct keys: structKeyExists false, but ENUMERABLE (Lucee parity)");

// Lucee 7 model (verified, and see GH #268): "a NULL value is the same as not
// existing in CFML" applies ONLY to structKeyExists — the key is still
// ENUMERABLE. structCount / structKeyArray / structKeyList / for-in all include
// a null-valued key; only structKeyExists reports it absent. This must hold
// whether the null was assigned via a struct LITERAL, MEMBER access
// (`s.x = nullValue()`), or BRACKET access (`s["x"] = nullValue()`) — all three
// are the same operation and must agree (GH #268: the member form used to drop
// the key, diverging from the literal/bracket forms and from Lucee).
//
// The historical "drop the key" behaviour existed to protect a Preside pattern
// (`for(field in record){ var v = record[field] }` over a query row with a NULL
// column). That rationale no longer holds: query rows store NULL columns as ""
// (a real value), so no genuine null keys appear there; and structKeyExists
// still guards the `if(StructKeyExists(page,prop)){ ... }` read path.

// --- member-access assignment (the GH #268 form) ---
s = {};
s.present = "x";
s.blank   = "";
s.nullkey = nullValue();

assertTrue(  "present key exists",              structKeyExists( s, "present" ) );
assertTrue(  "blank (empty-string) key exists", structKeyExists( s, "blank" ) );
assertFalse( "null-valued key does NOT exist",  structKeyExists( s, "nullkey" ) );
assertFalse( "missing key does not exist",      structKeyExists( s, "absent" ) );

// structKeyArray / structKeyList / structCount INCLUDE the null key.
keys = structKeyArray( s );
assertTrue( "keyArray includes null key",     arrayFindNoCase( keys, "nullkey" ) > 0 );
assertTrue( "keyArray includes present key",  arrayFindNoCase( keys, "present" ) > 0 );
assert(     "structCount includes null key",  structCount( s ), 3 );

// for-in enumerates the null key.
seen = "";
for ( var k in s ) { seen = listAppend( seen, k ); }
assertTrue( "for-in enumerates null key",  listFindNoCase( seen, "nullkey" ) > 0 );
assert(     "for-in count includes null key", listLen( seen ), 3 );

// --- bracket-access assignment must agree with the member form ---
t = {};
t["a"] = 1;
t["x"] = nullValue();
t["b"] = 2;
assert(      "bracket form: structCount includes null key", structCount( t ), 3 );
assertFalse( "bracket form: structKeyExists false for null key", structKeyExists( t, "x" ) );

suiteEnd();
</cfscript>
