<cfscript>
/*
 * duplicate( object, deepCopy=true ) — the SECOND argument.
 *
 * Lucee's signature is `duplicate( object, deepCopy )` with deepCopy defaulting
 * to TRUE. Passing `false` asks for a ONE-LEVEL copy: the top-level container is
 * new, but every value inside it stays shared by reference.
 *
 * RustCFML used to read only the first argument and always deep-copy, so
 * `duplicate( x, false )` silently produced a full deep clone of the whole
 * object graph. Preside's HandlerService._newHandlerBean() does exactly
 * `Duplicate( variables._ehBean, false )` on a hot path.
 *
 * Every expectation below was measured against Lucee 7.0.4.34 before being
 * written down.
 */
suiteBegin( "duplicate() deepCopy flag" );

// ---- deep (default and explicit true): nothing is shared -------------------
dupflagDeepDefault = { n = { v = 1 } };
dupflagCopy = duplicate( dupflagDeepDefault );
dupflagCopy.n.v = 99;
assert( "one-arg duplicate deep-copies a nested struct", dupflagDeepDefault.n.v, 1 );

dupflagDeepTrue = { n = { v = 1 } };
dupflagCopy = duplicate( dupflagDeepTrue, true );
dupflagCopy.n.v = 99;
assert( "duplicate(x,true) deep-copies a nested struct", dupflagDeepTrue.n.v, 1 );

dupflagDeepArr = { a = [ 1, 2 ] };
dupflagCopy = duplicate( dupflagDeepArr );
dupflagCopy.a[ 1 ] = 99;
assert( "one-arg duplicate deep-copies a nested array", dupflagDeepArr.a[ 1 ], 1 );

// ---- shallow (false): the top level is new, the contents are shared --------
dupflagShallow = { n = { v = 1 } };
dupflagCopy = duplicate( dupflagShallow, false );
dupflagCopy.n.v = 99;
assert( "duplicate(x,false) SHARES a nested struct", dupflagShallow.n.v, 99 );

dupflagShallow = { a = { b = { c = 1 } } };
dupflagCopy = duplicate( dupflagShallow, false );
dupflagCopy.a.b.c = 99;
assert( "duplicate(x,false) shares three levels down", dupflagShallow.a.b.c, 99 );

dupflagShallow = { a = [ 1, 2 ] };
dupflagCopy = duplicate( dupflagShallow, false );
dupflagCopy.a[ 1 ] = 99;
assert( "duplicate(x,false) SHARES a nested array", dupflagShallow.a[ 1 ], 99 );

// ...but the top-level container really is independent.
dupflagShallow = { a = 1 };
dupflagCopy = duplicate( dupflagShallow, false );
dupflagCopy.newkey = 2;
assertFalse( "duplicate(x,false) top level: adding a key does not touch the source", structKeyExists( dupflagShallow, "newkey" ) );

dupflagShallow = { a = 1 };
dupflagCopy = duplicate( dupflagShallow, false );
dupflagCopy.a = 2;
assert( "duplicate(x,false) top level: overwriting a key does not touch the source", dupflagShallow.a, 1 );

// ---- array roots behave the same way --------------------------------------
dupflagArrRoot = [ [ 1, 2 ] ];
dupflagCopy = duplicate( dupflagArrRoot, false );
dupflagCopy[ 1 ][ 1 ] = 99;
assert( "duplicate(array,false) shares the nested array", dupflagArrRoot[ 1 ][ 1 ], 99 );

dupflagArrRoot = [ 1, 2 ];
dupflagCopy = duplicate( dupflagArrRoot, false );
dupflagCopy[ 1 ] = 99;
assert( "duplicate(array,false) copies the top-level array", dupflagArrRoot[ 1 ], 1 );

// ---- the flag coerces like any CFML boolean -------------------------------
dupflagShallow = { n = { v = 1 } };
dupflagCopy = duplicate( dupflagShallow, "no" );
dupflagCopy.n.v = 99;
assert( "duplicate(x,'no') is treated as false", dupflagShallow.n.v, 99 );

// ---- member-function forms honour the flag too ----------------------------
dupflagShallow = { n = { v = 1 } };
dupflagCopy = dupflagShallow.duplicate( false );
dupflagCopy.n.v = 99;
assert( "struct.duplicate(false) shares the nested struct", dupflagShallow.n.v, 99 );

dupflagShallow = { n = { v = 1 } };
dupflagCopy = dupflagShallow.duplicate();
dupflagCopy.n.v = 99;
assert( "struct.duplicate() deep-copies the nested struct", dupflagShallow.n.v, 1 );

dupflagArrRoot = [ [ 1 ] ];
dupflagCopy = dupflagArrRoot.duplicate( false );
dupflagCopy[ 1 ][ 1 ] = 99;
assert( "array.duplicate(false) shares the nested array", dupflagArrRoot[ 1 ][ 1 ], 99 );

// ---- a nested COMPONENT is a reference under false, a copy under deep -----
dupflagStruct = { c = createObject( "component", "core.DuplicateFlagFixture" ) };
dupflagCopy = duplicate( dupflagStruct, false );
dupflagCopy.c.setMarker( "changed" );
assert( "duplicate(x,false) shares a nested component", dupflagStruct.c.getMarker(), "changed" );

dupflagStruct = { c = createObject( "component", "core.DuplicateFlagFixture" ) };
dupflagCopy = duplicate( dupflagStruct, true );
dupflagCopy.c.setMarker( "changed" );
assert( "duplicate(x,true) deep-copies a nested component", dupflagStruct.c.getMarker(), "orig" );

dupflagStruct = { c = createObject( "component", "core.DuplicateFlagFixture" ) };
dupflagCopy = duplicate( dupflagStruct );
dupflagCopy.c.setMarker( "changed" );
assert( "one-arg duplicate deep-copies a nested component", dupflagStruct.c.getMarker(), "orig" );

// A component at the ROOT is the top-level container, so it is copied in BOTH
// modes.
dupflagComp = createObject( "component", "core.DuplicateFlagFixture" );
dupflagCopy = duplicate( dupflagComp, false );
dupflagCopy.setMarker( "changed" );
assert( "duplicate(component,false) still copies the root component", dupflagComp.getMarker(), "orig" );

dupflagComp = createObject( "component", "core.DuplicateFlagFixture" );
dupflagCopy = duplicate( dupflagComp );
dupflagCopy.setMarker( "changed" );
assert( "one-arg duplicate copies the root component", dupflagComp.getMarker(), "orig" );

// ---- queries ---------------------------------------------------------------
dupflagQuery = queryNew( "id,name", "integer,varchar", [ [ 1, "a" ], [ 2, "b" ] ] );
dupflagStruct = { q = dupflagQuery };
dupflagCopy = duplicate( dupflagStruct, false );
querySetCell( dupflagCopy.q, "name", "ZZZ", 1 );
assert( "duplicate(x,false) SHARES a nested query", dupflagQuery.name[ 1 ], "ZZZ" );

dupflagQuery = queryNew( "id,name", "integer,varchar", [ [ 1, "a" ], [ 2, "b" ] ] );
dupflagStruct = { q = dupflagQuery };
dupflagCopy = duplicate( dupflagStruct );
querySetCell( dupflagCopy.q, "name", "ZZZ", 1 );
assert( "one-arg duplicate deep-copies a nested query", dupflagQuery.name[ 1 ], "a" );

dupflagQuery = queryNew( "id,name", "integer,varchar", [ [ 1, "a" ] ] );
dupflagCopy = duplicate( dupflagQuery, false );
querySetCell( dupflagCopy, "name", "ZZZ", 1 );
assert( "duplicate(query,false) copies the root query", dupflagQuery.name[ 1 ], "a" );

// ---- closures survive both modes ------------------------------------------
dupflagStruct = { f = function() { return 42; } };
dupflagCopy = duplicate( dupflagStruct );
assert( "one-arg duplicate keeps a nested closure callable", dupflagCopy.f(), 42 );

dupflagStruct = { f = function() { return 42; } };
dupflagCopy = duplicate( dupflagStruct, false );
assert( "duplicate(x,false) keeps a nested closure callable", dupflagCopy.f(), 42 );

// ---- deep copy still preserves internal aliasing and survives cycles ------
dupflagShared = { v = 1 };
dupflagStruct = { x = dupflagShared, y = dupflagShared };
dupflagCopy = duplicate( dupflagStruct, true );
dupflagCopy.x.v = 99;
assert( "deep duplicate keeps an internally shared struct shared", dupflagCopy.y.v, 99 );

dupflagCyclic = { v = 1 };
dupflagCyclic.self = dupflagCyclic;
dupflagCopy = duplicate( dupflagCyclic, true );
dupflagCopy.v = 5;
assert( "deep duplicate terminates on a circular reference", dupflagCopy.self.v, 5 );

suiteEnd();
</cfscript>
