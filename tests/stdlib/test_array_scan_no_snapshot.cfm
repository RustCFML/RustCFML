<cfscript>
// The array scan BIFs used to walk `CfmlArray::iter()`, which is `snapshot()` —
// a clone of the whole backing Vec before the first element was looked at. A
// find that matched at index 1 still paid for all N, and a find over an EMPTY
// array still paid for the call. They now walk the live array under a read
// guard, which is only safe while the comparison cannot re-enter the same
// array's lock — so these pin the shapes that decide that: a complex needle, a
// self-referential array, and a needle that is the array itself.
suiteBegin("Array scans without snapshotting");

// --- the ordinary scalar cases ------------------------------------------
a = [ "alpha", "Beta", "gamma", "Beta" ];
assert( "arrayFind exact"            , arrayFind( a, "gamma" )           , 3 );
assert( "arrayFind is case-sensitive", arrayFind( a, "beta" )            , 0 );
assert( "arrayFindNoCase"            , arrayFindNoCase( a, "beta" )      , 2 );
assert( "arrayFind misses"           , arrayFind( a, "nope" )            , 0 );
assert( "arrayFindAll"               , arrayToList( arrayFindAll( a, "Beta" ) )      , "2,4" );
assert( "arrayFindAllNoCase"         , arrayToList( arrayFindAllNoCase( a, "beta" ) ), "2,4" );
assert( "member findNoCase"          , a.findNoCase( "GAMMA" )           , 3 );

// an empty array answers without touching anything
assert( "arrayFind over empty"       , arrayFind( [], "x" )              , 0 );
assert( "arrayFindAll over empty"    , arrayLen( arrayFindAll( [], "x" ) ), 0 );

// numbers and booleans still compare by their string form
assert( "arrayFind number"           , arrayFind( [ 1, 2, 3 ], 2 )       , 2 );
assert( "arrayFind 1 does not match 1.0", arrayFind( [ "1.0" ], 1 )      , 0 );
assert( "arrayFind boolean"          , arrayFind( [ true, false ], false ), 2 );

// --- COMPLEX needles take the fall-back walk ----------------------------
target  = { id=2 };
structs = [ { id=1 }, { id=2 }, { id=3 } ];
assert( "arrayFind struct needle compares by content", arrayFind( structs, target ), 2 );
assert( "arrayFind array needle", arrayFind( [ [1,2], [3,4] ], [ 3, 4 ] ), 2 );
assert( "a struct never equals a scalar", arrayFind( structs, "x" ), 0 );

// --- re-entrancy: a scan must not deadlock on a self-referential array ---
selfRef = [ "a" ];
arrayAppend( selfRef, selfRef );
assert( "scalar needle over a self-referential array", arrayFind( selfRef, "a" ), 1 );
assert( "scalar miss over a self-referential array"  , arrayFind( selfRef, "zz" ), 0 );
assert( "the array finds ITSELF by identity"         , arrayFind( selfRef, selfRef ), 2 );

// --- the reducers walk the live array too -------------------------------
nums = [ 4, 1, 9, 3 ];
assert( "arrayMin", arrayMin( nums ), 1 );
assert( "arrayMax", arrayMax( nums ), 9 );
assert( "arraySum", arraySum( nums ), 17 );
assert( "arrayAvg", arrayAvg( nums ), 4.25 );
assert( "arrayMin over empty", arrayMin( [] ), 0 );
assert( "arrayMax over empty", arrayMax( [] ), 0 );

// --- substring-match contains -------------------------------------------
assert( "arrayContains substring"      , arrayContains( [ "foobar", "baz" ], "oob", true ), 1 );
assert( "arrayContainsNoCase substring", arrayContainsNoCase( [ "FooBar", "baz" ], "oob", true ), 1 );

suiteEnd();
</cfscript>
