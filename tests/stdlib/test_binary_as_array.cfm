<!--- GH #340 — a binary IS a Java `byte[]`.

     On Lucee `binaryDecode(...)` yields a native byte[], so the array BIFs
     operate on it directly and its elements are SIGNED bytes (0xFF reads back
     as -1, not 255). This engine used to answer every one of those with an
     empty/false/zero value: `arrayLen( bytes )` returned 0 for a perfectly good
     payload, so a guard like `if ( arrayLen( bytes ) )` took the wrong branch
     with nothing thrown anywhere.

     Every assertion below is measured against Lucee 7.1.0.204. --->
<cfscript>
suiteBegin("Binary as byte array (GH ##340)");

b   = binaryDecode( "QUJD", "base64" );   // "ABC" -> 65, 66, 67
neg = binaryDecode( "//8=", "base64" );   // 0xFF 0xFF -> signed -1, -1

// --- the core three the issue reported ---
assert("arrayLen(binary) counts bytes", arrayLen( b ), 3);
assertTrue("isArray(binary) is true", isArray( b ));
assert("binary[1] reads byte 1", b[1], 65);
assert("binary[3] reads the last byte", b[3], 67);

// A binary is BOTH an array and a binary — the two are not exclusive on Lucee,
// so an existing isBinary() guard keeps working.
assertTrue("isBinary(binary) is still true", isBinary( b ));
assertFalse("isSimpleValue(binary) is false", isSimpleValue( b ));
assert("len(binary) still agrees", len( b ), 3);

// --- signed, not unsigned: the part that is easy to get wrong ---
assert("high bytes read back SIGNED", arrayLen( neg ) & ":" & neg[1] & "," & neg[2], "2:-1,-1");

// --- the read-only array family ---
assert("arrayToList", arrayToList( b ), "65,66,67");
assert("arraySlice", arrayToList( arraySlice( b, 2, 2 ) ), "66,67");
assert("arrayMid", arrayToList( arrayMid( b, 2, 2 ) ), "66,67");
assert("arrayFind", arrayFind( b, 66 ), 2);
assert("arrayReverse", arrayToList( arrayReverse( b ) ), "67,66,65");
assert("arrayFirst", arrayFirst( b ), 65);
assert("arrayLast", arrayLast( b ), 67);
assertFalse("arrayIsEmpty", arrayIsEmpty( b ));
assertTrue("arrayIsDefined", arrayIsDefined( b, 2 ));
assertTrue("arrayIndexExists", arrayIndexExists( b, 3 ));
assert("arrayMin", arrayMin( b ), 65);
assert("arrayMax", arrayMax( b ), 67);
assert("arraySum", arraySum( b ), 198);
assert("arrayAvg", arrayAvg( b ), 66);
assert("arrayToStruct", serializeJSON( arrayToStruct( b ) ), '{"1":65,"2":66,"3":67}');
assert("arrayMerge", arrayToList( arrayMerge( b, [ 9 ] ) ), "65,66,67,9");

// --- iteration: for-in and the higher-order BIFs ---
_forIn = "";
for ( _byte in b ) { _forIn = listAppend( _forIn, _byte ); }
assert("for-in iterates the bytes", _forIn, "65,66,67");

assert("arrayMap", arrayToList( arrayMap( b, function( x ) { return x * 2; } ) ), "130,132,134");
assert("arrayFilter", arrayToList( arrayFilter( b, function( x ) { return x GT 65; } ) ), "66,67");
assert("arrayReduce", arrayReduce( b, function( acc, x ) { return acc + x; }, 0 ), 198);

request._binEach = "";
arrayEach( b, function( x ) { request._binEach = listAppend( request._binEach, x ); } );
assert("arrayEach", request._binEach, "65,66,67");

// --- out of range throws rather than reading back empty ---
// This is the whole point of the fix: a silent empty answer for a real payload
// is the dangerous failure. It must also be CATCHABLE in the same frame — the
// index op is dispatched with `?`, so a bare Err would skip an enclosing try{}.
assertThrows("binary index out of range throws", function() { return b[ 4 ]; });

suiteEnd();
</cfscript>
