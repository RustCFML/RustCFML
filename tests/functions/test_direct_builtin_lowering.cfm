<cfscript>
/*
 * v0.614.0 — type/existence predicates are lowered by codegen to a
 * compile-time-bound `CallBuiltin` op instead of `LoadGlobal` + `Call` (the
 * shape Lucee emits; see `VariableImpl._writeOutFirstBIF`). These assert the
 * lowering is SEMANTICS-PRESERVING — especially the cases whose logic lives in
 * the builtin body and would be lost if the op were ever "simplified" into a
 * naive inline type check.
 */
suiteBegin( "Direct builtin lowering (CallBuiltin)" );

s   = { a = 1, nul = nullValue() };
arr = [ 1, 2 ];

assert( "isArray on array",      isArray( arr ),        true  );
assert( "isArray on struct",     isArray( s ),          false );
assert( "isStruct on struct",    isStruct( s ),         true  );
assert( "isStruct on array",     isStruct( arr ),       false );
assert( "isBoolean yes",         isBoolean( "yes" ),    true  );
assert( "isBoolean word",        isBoolean( "banana" ), false );
assert( "isNumeric numeric str", isNumeric( "12" ),     true  );
assert( "isNumeric word",        isNumeric( "abc" ),    false );
assert( "isSimpleValue string",  isSimpleValue( "x" ),  true  );
assert( "isSimpleValue array",   isSimpleValue( arr ),  false );

// structKeyExists keeps its Lucee-parity rules: case-insensitive lookup, and a
// key holding NULL reports as ABSENT.
assert( "ske present",           structKeyExists( s, "a" ),   true  );
assert( "ske case-insensitive",  structKeyExists( s, "A" ),   true  );
assert( "ske null = absent",     structKeyExists( s, "nul" ), false );
assert( "ske missing",           structKeyExists( s, "zz" ),  false );

// The `arguments` scope is a HYBRID — array AND struct. That rule lives in
// fn_is_array/fn_is_struct, so it only survives if the op calls them.
function hybrid( a ) {
    return isArray( arguments ) & "/" & isStruct( arguments );
}
assert( "arguments is array AND struct", hybrid( 1 ), "true/true" );

// An argument that is itself a call must still evaluate before the predicate.
function two() { return [ 1, 2 ]; }
assert( "nested call arg", isArray( two() ), true );

// Named arguments are never lowered — they must still route through CallNamed.
assert( "named arg form", structKeyExists( struct = s, key = "a" ), true );

// ── pure string / list / regex helpers, added to the allowlist after the
//    predicates. These have REAL bodies (unlike the predicates), so they also
//    guard against the op ever being "optimised" into something that skips the
//    builtin.
assert( "len string",        len( "abcd" ),                     4 );
assert( "len array",         len( [ 1, 2, 3 ] ),                3 );
assert( "trim",              trim( "  hi  " ),                  "hi" );
assert( "lcase",             lcase( "AbC" ),                    "abc" );
assert( "ucase",             ucase( "AbC" ),                    "ABC" );
assert( "arrayLen",          arrayLen( [ 1, 2 ] ),              2 );
assert( "listLen",           listLen( "a,b,c" ),                3 );
assert( "listFirst",         listFirst( "a,b,c" ),              "a" );
assert( "listRest",          listRest( "a,b,c" ),               "b,c" );
assert( "listLen delim",     listLen( "a|b|c", "|" ),           3 );
assert( "listFirst delim",   listFirst( "a|b|c", "|" ),         "a" );
assert( "refind hit",        reFind( "[0-9]+", "ab123" ),       3 );
assert( "refind miss",       reFind( "[0-9]+", "abc" ),         0 );
assert( "refindNoCase",      reFindNoCase( "ABC", "xxabc" ),    3 );
assert( "replace once",      replace( "a-a-a", "a", "b" ),      "b-a-a" );
assert( "replace all",       replace( "a-a-a", "a", "b", "all" ), "b-b-b" );

// arrayFindNoCase is VM-INTERCEPTED and must NOT be lowered — if it ever is,
// it loses its interception. This asserts it still behaves.
letters = [ "Alpha", "Beta" ];
assert( "arrayFindNoCase (intercepted)", arrayFindNoCase( letters, "beta" ), 2 );

suiteEnd();
</cfscript>
