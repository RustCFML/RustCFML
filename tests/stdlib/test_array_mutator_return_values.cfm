<cfscript>
// GH #430 — the STANDALONE mutating array BIFs return a status value, while the
// MEMBER form returns the receiver so it can be chained. Measured against
// Lucee 7.1.0.204; every assertion below is engine-agnostic.
suiteBegin( "Array mutator return values (GH ##430)" );

// ---- standalone: `true` -----------------------------------------------------
a = [ 1, 2 ];  assert( "arrayAppend returns true",    arrayAppend( a, 9 ),        true );
a = [ 1, 2 ];  assert( "arrayPrepend returns true",   arrayPrepend( a, 9 ),       true );
a = [ 1, 2 ];  assert( "arrayClear returns true",     arrayClear( a ),            true );
a = [ 1, 2 ];  assert( "arrayDeleteAt returns true",  arrayDeleteAt( a, 1 ),      true );
a = [ 1, 2 ];  assert( "arrayDelete returns true",    arrayDelete( a, 1 ),        true );
a = [ 1, 2 ];  assert( "arraySet returns true",       arraySet( a, 1, 2, 0 ),     true );
a = [ 1, 2 ];  assert( "arrayResize returns true",    arrayResize( a, 5 ),        true );
a = [ 1, 2 ];  assert( "arrayInsertAt returns true",  arrayInsertAt( a, 1, 9 ),   true );
a = [ 2, 1 ];  assert( "arraySort returns true",      arraySort( a, "numeric" ),  true );
a = [ 2, 1 ];  assert( "arraySort(fn) returns true",  arraySort( a, function( x, y ){ return x - y; } ), true );
a = [ 1, 2 ];  assert( "arraySwap returns true",      arraySwap( a, 1, 2 ),       true );
a = [ 1, 2 ];  assert( "arrayAppend(merge) true",     arrayAppend( a, [ 3, 4 ], true ), true );

// ---- standalone: arrayPush/arrayUnshift return the NEW LENGTH ---------------
a = [ 1, 2 ];  assert( "arrayPush returns new length",    arrayPush( a, 9 ),    3 );
a = [ 1, 2 ];  assert( "arrayUnshift returns new length", arrayUnshift( a, 9 ), 3 );
a = [];        assert( "arrayPush onto empty",            arrayPush( a, 9 ),    1 );

// ---- the mutation itself still lands, status return or not ------------------
a = [ 1, 2 ];  arrayAppend( a, 9 );      assert( "append mutates",     arrayToList( a ), "1,2,9" );
a = [ 1, 2 ];  arrayPrepend( a, 9 );     assert( "prepend mutates",    arrayToList( a ), "9,1,2" );
a = [ 1, 2 ];  arrayPush( a, 9 );        assert( "push mutates",       arrayToList( a ), "1,2,9" );
a = [ 1, 2 ];  arrayUnshift( a, 9 );     assert( "unshift mutates",    arrayToList( a ), "9,1,2" );
a = [ 1, 2 ];  arrayClear( a );          assert( "clear mutates",      arrayLen( a ),    0 );
a = [ 1, 2 ];  arrayDeleteAt( a, 1 );    assert( "deleteAt mutates",   arrayToList( a ), "2" );
a = [ 1, 2 ];  arrayInsertAt( a, 1, 9 ); assert( "insertAt mutates",   arrayToList( a ), "9,1,2" );
a = [ 2, 1 ];  arraySort( a, "numeric" );assert( "sort mutates",       arrayToList( a ), "1,2" );
a = [ 1, 2 ];  arraySwap( a, 1, 2 );     assert( "swap mutates",       arrayToList( a ), "2,1" );
a = [ 1, 2 ];  arraySet( a, 1, 2, 0 );   assert( "set mutates",        arrayToList( a ), "0,0" );
a = [ 1, 2 ];  arrayResize( a, 4 );      assert( "resize mutates",     arrayLen( a ),    4 );

// A mutation through an alias is visible in both handles — the write-back that
// used to store the BIF's return value over the argument is gone, so this is
// the only thing keeping the caller's variable current.
a = [ 1, 2 ]; b = a; arrayAppend( a, 9 );
assert( "alias sees the append", arrayToList( b ), "1,2,9" );

// Nested container, no simple identifier to write back to.
o = { list = [ 1, 2 ] }; arrayAppend( o.list, 9 );
assert( "nested append mutates", arrayToList( o.list ), "1,2,9" );

// Inside a function (the fused-op path) and through an argument.
function appendInside() { var loc = [ 1, 2 ]; arrayAppend( loc, 9 ); return arrayToList( loc ); }
assert( "append inside a function", appendInside(), "1,2,9" );
function appendArg( x ) { arrayAppend( x, 9 ); }
a = [ 1, 2 ]; appendArg( a ); assert( "append through an argument", arrayToList( a ), "1,2,9" );

// ---- member form returns the RECEIVER (chainable) ---------------------------
a = [ 1, 2 ];  assert( "a.append returns the array",   arrayToList( a.append( 9 ) ),      "1,2,9" );
a = [ 1, 2 ];  assert( "a.prepend returns the array",  arrayToList( a.prepend( 9 ) ),     "9,1,2" );
a = [ 1, 2 ];  assert( "a.clear returns the array",    arrayLen( a.clear() ),             0 );
a = [ 1, 2 ];  assert( "a.deleteAt returns the array", arrayToList( a.deleteAt( 1 ) ),    "2" );
a = [ 1, 2 ];  assert( "a.delete returns the array",   arrayToList( a.delete( 1 ) ),      "2" );
a = [ 1, 2 ];  assert( "a.insertAt returns the array", arrayToList( a.insertAt( 1, 9 ) ), "9,1,2" );
a = [ 2, 1 ];  assert( "a.sort returns the array",     arrayToList( a.sort( "numeric" ) ),"1,2" );
a = [ 1, 2 ];  assert( "a.swap returns the array",     arrayToList( a.swap( 1, 2 ) ),     "2,1" );
a = [ 1, 2 ];  assert( "a.set returns the array",      arrayToList( a.set( 1, 2, 0 ) ),   "0,0" );
a = [ 1, 2 ];  assert( "a.resize returns the array",   arrayLen( a.resize( 5 ) ),         5 );
a = [ "A", "b" ]; assert( "a.deleteNoCase returns the array", arrayToList( a.deleteNoCase( "a" ) ), "b" );
a = [ 1, 2 ];  assert( "member calls chain",           arrayToList( a.append( 3 ).append( 4 ) ), "1,2,3,4" );

// ...except the JS-named pair, which returns the new length in BOTH forms.
a = [ 1, 2 ];  assert( "a.push returns new length",    a.push( 9 ),    3 );
a = [ 1, 2 ];  assert( "a.unshift returns new length", a.unshift( 9 ), 3 );

// ...and the element-returning pair.
a = [ 1, 2 ];  assert( "a.pop returns the element",    a.pop(),        2 );
a = [ 1, 2 ];  assert( "a.shift returns the element",  a.shift(),      1 );

// ---- arrayReverse is PURE in both forms -------------------------------------
a = [ 1, 2 ];  assert( "arrayReverse returns reversed", arrayToList( arrayReverse( a ) ), "2,1" );
a = [ 1, 2 ];  arrayReverse( a );
assert( "arrayReverse leaves the original alone", arrayToList( a ), "1,2" );
a = [ 1, 2 ];  assert( "a.reverse returns reversed", arrayToList( a.reverse() ), "2,1" );
a = [ 1, 2 ];  a.reverse();
assert( "a.reverse leaves the receiver alone", arrayToList( a ), "1,2" );

// arrayMerge is pure too (the reversed/merged copy is a separate array).
a = [ 1, 2 ]; m = arrayMerge( a, [ 3 ] ); arrayAppend( m, 4 );
assert( "arrayMerge is pure", arrayToList( a ), "1,2" );

// ---- an out-of-range or uncastable position is REFUSED, not a silent no-op ---
// (GH ##430: these used to mutate the WRONG element — position 0 and "x" both
// landed on index 0 — or report success having done nothing.)
assertThrows( "insertAt past the end",   function(){ a = [ 1, 2 ]; arrayInsertAt( a, 4, 9 ); } );
assertThrows( "insertAt at 0",           function(){ a = [ 1, 2 ]; arrayInsertAt( a, 0, 9 ); } );
assertThrows( "insertAt negative",       function(){ a = [ 1, 2 ]; arrayInsertAt( a, -1, 9 ); } );
assertThrows( "insertAt non-numeric",    function(){ a = [ 1, 2 ]; arrayInsertAt( a, "x", 9 ); } );
assertThrows( "deleteAt past the end",   function(){ a = [ 1, 2 ]; arrayDeleteAt( a, 3 ); } );
assertThrows( "deleteAt at 0",           function(){ a = [ 1, 2 ]; arrayDeleteAt( a, 0 ); } );
assertThrows( "deleteAt non-numeric",    function(){ a = [ 1, 2 ]; arrayDeleteAt( a, "x" ); } );
assertThrows( "member deleteAt past end",function(){ a = [ 1, 2 ]; a.deleteAt( 3 ); } );
assertThrows( "swap first index 0",      function(){ a = [ 1, 2 ]; arraySwap( a, 0, 1 ); } );
assertThrows( "swap first index high",   function(){ a = [ 1, 2 ]; arraySwap( a, 3, 1 ); } );
assertThrows( "swap second index high",  function(){ a = [ 1, 2 ]; arraySwap( a, 1, 3 ); } );
assertThrows( "arraySet start 0",        function(){ a = [ 1, 2 ]; arraySet( a, 0, 2, "z" ); } );

// ...and the in-range edges still work.
a = [ 1, 2 ]; arrayInsertAt( a, 3, 9 ); assert( "insertAt at len+1 appends", arrayToList( a ), "1,2,9" );
a = [ 1, 2 ]; arrayDeleteAt( a, 2 );    assert( "deleteAt at len",          arrayToList( a ), "1" );
a = [ 1, 2 ]; arraySet( a, 3, 4, "z" ); assert( "arraySet past the end grows", arrayToList( a ), "1,2,z,z" );

// A resize that does not grow the array is a no-op. A NEGATIVE size used to
// become usize::MAX and hang the engine in the grow loop.
a = [ 1, 2, 3 ]; arrayResize( a, 1 );  assert( "resize smaller is a no-op", arrayToList( a ), "1,2,3" );
a = [ 1, 2 ];    assert( "resize negative returns true", arrayResize( a, -1 ), true );
assert( "resize negative leaves the array alone", arrayToList( a ), "1,2" );

// ---- arrayLen refuses a value that is not an array --------------------------
assertThrows( "arrayLen(boolean)", function(){ return arrayLen( true ); } );
assertThrows( "arrayLen(string)",  function(){ return arrayLen( "abc" ); } );
assertThrows( "arrayLen(number)",  function(){ return arrayLen( 5 ); } );
assert( "arrayLen(array) still counts", arrayLen( [ 1, 2 ] ), 2 );

// ---- a subscript of a scalar is an error, not null --------------------------
// This is what made `arrayAppend(a,x)[1]` read as "missing" instead of failing.
assertThrows( "boolean[1]",   function(){ x = true; return x[ 1 ]; } );
assertThrows( "number[1]",    function(){ x = 5;    return x[ 1 ]; } );
assertThrows( "boolean['k']", function(){ x = true; return x[ "k" ]; } );
assert( "string[1] is still a character", ( function(){ x = "abc"; return x[ 1 ]; } )(), "a" );

// ---- .each returns the receiver; the standalone BIF returns null ------------
a = [ 1, 2 ];
assert( "a.each returns the array",   arrayToList( a.each( function( i ){} ) ), "1,2" );
assert( "arrayEach returns null",     isNull( arrayEach( a, function( i ){} ) ), true );
st = { a = 1 };
assert( "s.each returns the struct",  structCount( st.each( function( k, v ){} ) ), 1 );
assert( "structEach returns null",    isNull( structEach( st, function( k, v ){} ) ), true );

suiteEnd();
</cfscript>
