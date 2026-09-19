<cfscript>
suiteBegin("Core: a numeric subscript on an undefined root vivifies a STRUCT (GH ##429)");

// Through v0.685.3 RustCFML vivified a 1-based auto-growing ARRAY for a numeric
// subscript on an undefined root (`u[2]=1` -> [null,1]) where Lucee builds a
// STRUCT (`{"2":1}`). The immediate read agrees on both engines, so the
// difference was invisible until something inspected the container -- isArray /
// isStruct, arrayLen vs structCount, serializeJSON, or a later arrayAppend.
//
// Lucee has no special "vivified" type: the result is an ordinary struct, and
// the array BIFs reach it through a POSITIONAL view over the keys "1".."n"
// (Lucee's StructAsArray). Every expectation below is measured on Lucee
// 7.1.0.204.
//
// PROBING TRAP: an unscoped name inside a closure writes to PAGE scope, so
// reusing a name across cases makes the second case see the first one's
// container. Every case below uses a fresh name.

// ---- 1. the container type ------------------------------------------------
function vivNumeric()  { n1[ 2 ] = 1; return { isArr = isArray( n1 ), isSt = isStruct( n1 ), json = serializeJSON( n1 ) }; }
_v = vivNumeric();
assert( "u[2]=1 is NOT an array", _v.isArr, false );
assert( "u[2]=1 is a struct",     _v.isSt,  true );
assert( "u[2]=1 keys by the subscript", _v.json, '{"2":1}' );

function vivOne()      { n2[ 1 ] = 1; return isStruct( n2 ) & ":" & structCount( n2 ); }
assert( "u[1]=1 is a struct too", vivOne(), "true:1" );

function vivDecimal()  { n3[ 2.5 ] = 1; return serializeJSON( n3 ); }
assert( "a decimal subscript keys as written", vivDecimal(), '{"2.5":1}' );

function vivString()   { n4[ "a" ] = 1; return isStruct( n4 ); }
assert( "a string key still vivifies a struct", vivString(), true );

function vivNested()   { n5.a[ 2 ] = 1; return isStruct( n5.a ) & ":" & serializeJSON( n5.a ); }
assert( "a nested undefined member vivifies a struct", vivNested(), 'true:{"2":1}' );

function vivDeep()     { n6[ 1 ][ 1 ] = 1; return serializeJSON( n6 ); }
assert( "a doubly-nested subscript nests structs", vivDeep(), '{"1":{"1":1}}' );

// The read still resolves, which is why this was invisible for so long.
function vivRead()     { n7[ 2 ] = 1; return n7[ 2 ]; }
assert( "the value reads back through the same subscript", vivRead(), 1 );

function vivKeyExists() { n8[ 2 ] = 1; return structKeyExists( n8, "2" ); }
assert( "the subscript is a real struct key", vivKeyExists(), true );

// The loop idiom that makes this reachable in ordinary code.
function buildByLoop() {
    for ( i = 1; i <= 3; i++ ) { n9[ i ] = i * 10; }
    return isStruct( n9 ) & ":" & structCount( n9 ) & ":" & n9[ 3 ];
}
assert( "a 1..n build loop produces a struct", buildByLoop(), "true:3:30" );

// ---- 2. the array BIFs see the positional view ----------------------------
// This is what keeps the loop idiom working: Lucee's array BIFs accept a
// numeric-keyed struct, so ours must too.
assert( "arrayLen sees the positional length", arrayLen( { "1": 10, "2": 20 } ), 2 );
assert( "arrayIsEmpty is false for a populated one", arrayIsEmpty( { "1": 10 } ), false );
assert( "arrayFirst reads position 1", arrayFirst( { "1": 10, "2": 20 } ), 10 );
assert( "arrayLast reads position n",  arrayLast(  { "1": 10, "2": 20 } ), 20 );
assert( "arrayToList walks positions", arrayToList( { "1": 10, "2": 20 } ), "10,20" );
assert( "arraySum adds the values",    arraySum(   { "1": 10, "2": 20 } ), 30 );
assert( "arrayAvg averages them",      arrayAvg(   { "1": 10, "2": 20 } ), 15 );
assert( "arrayMin",                    arrayMin(   { "1": 10, "2": 20 } ), 10 );
assert( "arrayMax",                    arrayMax(   { "1": 10, "2": 20 } ), 20 );
assert( "arrayReverse",  serializeJSON( arrayReverse( { "1": 10, "2": 20 } ) ), "[20,10]" );
assert( "arraySlice",    serializeJSON( arraySlice(   { "1": 10, "2": 20 }, 1, 1 ) ), "[10]" );
assert( "arrayMerge",    serializeJSON( arrayMerge( { "1": 10 }, { "1": 20 } ) ), "[10,20]" );

// Higher-order BIFs go through the VM intercept seam, not the stdlib one.
assert( "arrayMap",    serializeJSON( arrayMap(    { "1": 10, "2": 20 }, function( v ){ return v + 1; } ) ), "[11,21]" );
assert( "arrayFilter", serializeJSON( arrayFilter( { "1": 10, "2": 20 }, function( v ){ return v > 10; } ) ), "[20]" );
assert( "arraySome",   arraySome(  { "1": 10 }, function( v ){ return v == 10; } ), true );
assert( "arrayEvery",  arrayEvery( { "1": 10 }, function( v ){ return v == 10; } ), true );

function eachWalksPositions() {
    var out = "";
    arrayEach( { "1": "a", "2": "b" }, function( v ){ out &= v; } );
    return out;
}
assert( "arrayEach walks the positions", eachWalksPositions(), "ab" );

// ---- 3. mutating BIFs keep it a struct ------------------------------------
// The regression that matters most: before this, arrayAppend REPLACED the
// struct with a one-element array, silently dropping everything in it.
function appendKeepsStruct() {
    var s = { "1": 10, "2": 20 };
    arrayAppend( s, 30 );
    return isStruct( s ) & ":" & serializeJSON( s );
}
assert( "arrayAppend appends key n+1 and keeps the struct", appendKeepsStruct(),
        'true:{"1":10,"2":20,"3":30}' );

function appendToVivified() {
    n10[ 1 ] = "a";
    arrayAppend( n10, "b" );
    return serializeJSON( n10 );
}
assert( "arrayAppend onto a vivified container", appendToVivified(), '{"1":"a","2":"b"}' );

function prependRenumbers() {
    var s = { "1": 10, "2": 20 };
    arrayPrepend( s, 5 );
    return serializeJSON( s );
}
assert( "arrayPrepend inserts at 1 and renumbers", prependRenumbers(),
        '{"1":5,"2":10,"3":20}' );

// ---- 3b. SPARSE struct-as-array: keys, not positions -----------------------
// The bug that shipped in v0.685.4: the append key was computed as
// positional-size + 1, which coincides with the correct answer ONLY for a dense
// 1..n struct. Every test above used a dense one, so it passed while `u[2]=1;
// arrayAppend(u,5)` -- the headline shape of this very issue -- produced
// {"2":5}, OVERWRITING the element. Lucee's rule is max-numeric-key + 1.
function appendSparse()      { var s = {}; s[ 2 ] = 1; arrayAppend( s, 5 ); return serializeJSON( s ); }
assert( "append onto {2:1} adds key 3, keeping the element", appendSparse(), '{"2":1,"3":5}' );

function appendSparse5()     { var s = {}; s[ 5 ] = 1; arrayAppend( s, 9 ); return serializeJSON( s ); }
assert( "append onto {5:1} adds key 6", appendSparse5(), '{"5":1,"6":9}' );

function appendGap()         { var s = {}; s[ 1 ] = 1; s[ 5 ] = 2; arrayAppend( s, 9 ); return serializeJSON( s ); }
assert( "append past a gap uses the MAX key, not the count", appendGap(), '{"1":1,"5":2,"6":9}' );

function appendEmpty()       { var s = {}; arrayAppend( s, 9 ); return serializeJSON( s ); }
assert( "append onto {} starts at key 1", appendEmpty(), '{"1":9}' );

function appendZeroKey()     { var s = {}; s[ 0 ] = 1; arrayAppend( s, 9 ); return serializeJSON( s ); }
assert( "a non-positive max still appends at key 1", appendZeroKey(), '{"0":1,"1":9}' );

function appendNegKey()      { var s = {}; s[ -1 ] = 1; arrayAppend( s, 9 ); return serializeJSON( s ); }
assert( "a negative max still appends at key 1", appendNegKey(), '{"-1":1,"1":9}' );

// arrayPrepend shifts every existing key up by one. Renumbering densely to 1..n
// instead DROPPED the shifted value on a sparse struct.
function prependSparse()     { var s = {}; s[ 2 ] = 1; arrayPrepend( s, 9 ); return serializeJSON( s ); }
assert( "prepend onto {2:1} shifts it to key 3", prependSparse(), '{"1":9,"3":1}' );

function prependSparse5()    { var s = {}; s[ 5 ] = 1; arrayPrepend( s, 9 ); return serializeJSON( s ); }
assert( "prepend onto {5:1} shifts it to key 6", prependSparse5(), '{"1":9,"6":1}' );

// A removal names a POSITION key and refuses one that is absent, rather than
// silently doing nothing; and it does not renumber what remains.
throwsWithLater( "pop on a sparse struct refuses the missing key",
    function(){ var s = {}; s[ 2 ] = 1; return arrayPop( s ); },
    "can't remove key [1] from struct, key does not exist" );
throwsWithLater( "shift on a sparse struct refuses the missing key",
    function(){ var s = {}; s[ 2 ] = 1; return arrayShift( s ); },
    "can't remove key [1] from struct, key does not exist" );

function shiftDense()        { var s = { "1": 10, "2": 20 }; var r = arrayShift( s ); return r & "/" & serializeJSON( s ); }
assert( "shift removes key 1 and does NOT renumber", shiftDense(), '10/{"2":20}' );

function popDense()          { var s = { "1": 10, "2": 20 }; var r = arrayPop( s ); return r & "/" & serializeJSON( s ); }
assert( "pop removes the last position", popDense(), '20/{"1":10}' );

// The read-only positional view is unchanged by any of this: position 1 of
// {2:1} does not exist, so the view is a single empty slot (Lucee agrees).
function sparseToList()      { var s = {}; s[ 2 ] = 1; return "[" & arrayToList( s ) & "]"; }
assert( "the positional view of {2:1} is one empty slot", sparseToList(), "[]" );

function sparseLen()         { var s = {}; s[ 2 ] = 1; return arrayLen( s ); }
assert( "arrayLen of {2:1} is the struct size", sparseLen(), 1 );

// ---- 4. a non-numeric key is a cast error, not a silent zero --------------
function throwsWithLater( required string label, required callback, required string expected ) {
    try { callback(); assert( arguments.label, "no exception", arguments.expected ); }
    catch( any e ) { assert( arguments.label, e.message, arguments.expected ); }
}
function throwsWith( required string label, required callback, required string expected ) {
    try { callback(); assert( arguments.label, "no exception", arguments.expected ); }
    catch( any e ) { assert( arguments.label, e.message, arguments.expected ); }
}
throwsWith( "arrayLen on a non-numeric struct throws",
            function(){ return arrayLen( { a = 1 } ); },
            "can't cast struct to an array, key [A] is not a number" );
throwsWith( "arrayAppend on a non-numeric struct throws",
            function(){ var s = { a = 1 }; arrayAppend( s, 9 ); },
            "can't cast struct to an array, key [A] is not a number" );
throwsWith( "arraySort refuses a struct outright",
            function(){ return arraySort( { "1": 2, "2": 1 }, "numeric" ); },
            "Invalid call of the function [ArraySort], first Argument [array] is invalid, cannot sort object from type [struct]" );

// ---- 5. the arguments scope is NOT re-viewed ------------------------------
// `arguments` is a hybrid array/struct with marker keys and its own per-BIF
// handling; routing it through the generic struct view broke serializeJSON.
function argCount() { return arrayLen( arguments ); }
assert( "arrayLen(arguments) still counts bound args", argCount( 1, 2, 3 ), 3 );

function argSerialize() { return serializeJSON( arguments ); }
assert( "serializeJSON(arguments) still works", argSerialize( 1 ), '{"1":1}' );

suiteEnd();
</cfscript>
