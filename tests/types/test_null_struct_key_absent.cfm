<cfscript>
suiteBegin("Null struct keys are treated as absent (Lucee parity)");

// Lucee 7 (verified): "a NULL value is the same as not existing in CFML".
// A key whose value is null must be invisible to structKeyExists, for-in,
// structKeyArray/List and structCount. Regression: Preside SiteTreeService
// getPageProperty `if(StructKeyExists(page,prop)){ value=page[prop]; ... }` and
// PresideObjectViewService `for(field in record){ var v=record[field] }` both
// blew up ("Variable undefined") on a null property/column until this matched Lucee.

s = {};
s.present = "x";
s.blank   = "";
s.nullkey = nullValue();

// structKeyExists
assertTrue(  "present key exists",          structKeyExists( s, "present" ) );
assertTrue(  "blank (empty-string) key exists", structKeyExists( s, "blank" ) );
assertFalse( "null-valued key does NOT exist", structKeyExists( s, "nullkey" ) );
assertFalse( "missing key does not exist",   structKeyExists( s, "absent" ) );

// structKeyArray / structKeyList / structCount exclude the null key
keys = structKeyArray( s );
assertFalse( "keyArray excludes null key", arrayFindNoCase( keys, "nullkey" ) > 0 );
assertTrue(  "keyArray includes present key", arrayFindNoCase( keys, "present" ) > 0 );
assert(      "structCount excludes null key", structCount( s ), 2 );

// for-in skips the null key
seen = "";
for ( var k in s ) { seen = listAppend( seen, k ); }
assertFalse( "for-in skips null key", listFindNoCase( seen, "nullkey" ) > 0 );
assert(      "for-in count excludes null key", listLen( seen ), 2 );

suiteEnd();
</cfscript>
