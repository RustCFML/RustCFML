<cfscript>
// The `arguments` scope holds ONE ENTRY PER DECLARED PARAMETER, in declaration
// order, holding null when the caller omitted it — so it is countable and
// listable in full, while the existence checks still report an omitted
// parameter as absent. Verified against Lucee 7.1 (see docs/known-issues.md).
suiteBegin("Arguments scope shape (declared-but-omitted params)");

function shape3( a, b, c ) {
    var keys = "";
    for ( var k in arguments ) { keys = listAppend( keys, k ); }
    return {
          count     = structCount( arguments )
        , keyList   = structKeyList( arguments )
        , keyArray  = arrayToList( structKeyArray( arguments ) )
        , forIn     = keys
        , len       = arguments.len()
        , json      = serializeJSON( arguments )
        , existsB   = structKeyExists( arguments, "b" )
        , definedB  = isDefined( "arguments.b" )
        , isEmpty   = structIsEmpty( arguments )
        , copyCount = structCount( structCopy( arguments ) )
        , dupCount  = structCount( duplicate( arguments ) )
    };
}
r = shape3( 1 );
assert("omitted params are counted",            r.count,    3);
assert("omitted params are listed, in order",   r.keyList,  "a,b,c");
assert("structKeyArray lists them too",         r.keyArray, "a,b,c");
assert("for-in yields them",                    r.forIn,    "a,b,c");
assert("len() counts them",                     r.len,      3);
assert("serializeJSON emits them as null",      r.json,     '{"a":1,"b":null,"c":null}');
assert("structKeyExists is FALSE for an omitted param", r.existsB,  false);
assert("isDefined is FALSE for an omitted param",       r.definedB, false);
assert("the scope is not empty",                r.isEmpty,  false);
assert("structCopy carries the null keys",      r.copyCount, 3);
assert("duplicate carries the null keys",       r.dupCount,  3);

// structAppend copies all declared keys out.
function appendOut( a, b, c ) { var o = {}; structAppend( o, arguments ); return structKeyList( o ); }
assert("structAppend carries the null keys", appendOut( 1 ), "a,b,c");

// A DEFAULTED but unsupplied param is present in the scope from the start, and
// reads back as its default once the preamble has applied it.
function withDefault( a, b = "DEF", c ) {
    return { count = structCount( arguments ), keys = structKeyList( arguments ), b = arguments.b };
}
d = withDefault( 1 );
assert("a defaulted param is counted",       d.count, 3);
assert("a defaulted param is listed",        d.keys,  "a,b,c");
assert("a defaulted param reads its default", d.b,    "DEF");

// Defaults must still APPLY — the null entry must not read as "supplied".
function defaultsApply( a = "DA", b = "DB" ) { return a & "/" & b; }
assert("defaults still apply when omitted",      defaultsApply(),       "DA/DB");
assert("a supplied arg still beats its default", defaultsApply( "X" ),  "X/DB");

// Reading an omitted declared param yields NULL — it does not throw. It
// concatenates as "", `len()` is 0, and a plain assignment leaves the target
// null. (Lucee throws only when such a null is PASSED as an argument to another
// function, which is argument binding, not this read — a probe that funnels the
// value through a helper call mistakes one for the other.)
function readOmitted( a, b ) {
    return {
          concat     = "[" & arguments.b & "]"
        , len        = len( arguments.b )
        , isNull     = isNull( arguments.b )
        , positional = isNull( arguments[ 2 ] )
    };
}
ro = readOmitted( 1 );
assert("an omitted param concatenates as empty",    ro.concat,     "[]");
assert("len() of an omitted param is 0",            ro.len,        0);
assert("isNull() sees an omitted param as null",    ro.isNull,     true);
assert("arguments[N] for an omitted param is null", ro.positional, true);

// All supplied, and the paramless overflow shape, are unchanged.
function allSupplied( a, b, c ) { return structCount( arguments ) & ":" & structKeyList( arguments ); }
assert("all-supplied is unchanged", allSupplied( 1, 2, 3 ), "3:a,b,c");
function overflow() { return structCount( arguments ) & ":" & structKeyList( arguments ); }
assert("overflow positional args are unchanged", overflow( "x", "y" ), "2:1,2");

// argumentCollection forwarding must not let a null entry override the
// callee's own default.
function fwdCallee( a, b = "BDEF", c = "CDEF" ) { return a & "/" & b & "/" & c; }
function fwdCaller( a, b, c ) { return fwdCallee( argumentCollection = arguments ); }
assert("forwarded null args do not override callee defaults", fwdCaller( "AV" ), "AV/BDEF/CDEF");

// An omitted parameter must never SHADOW the rest of the resolution chain:
// a bare read of its name reaches `variables.<name>` (Lucee: local ->
// arguments -> variables, a null argument is "not here"). Two shapes that used
// to break it: an eager frame, and an eager frame that also writes back into
// `arguments` (which re-mirrors the scope into the frame's locals).
variables.shadowSvc = "FROM_VARIABLES";
function bareReadEager( required string directory, any shadowSvc ) { var n = structCount( arguments ); return shadowSvc; }
assert("omitted param does not shadow variables (eager frame)", bareReadEager( "/x" ), "FROM_VARIABLES");
function bareReadAfterArgStore( required string directory, any shadowSvc ) {
    var n = structCount( arguments );
    arguments.directory = replace( arguments.directory, "\", "/", "all" );
    return shadowSvc;
}
assert("omitted param does not shadow variables after an arguments.x= store", bareReadAfterArgStore( "/x" ), "FROM_VARIABLES");
function bareReadLazyAfterArgStore( required string directory, any shadowSvc ) { arguments.directory = "z"; return shadowSvc.len(); }
assert("method call on the resolved variables value works", bareReadLazyAfterArgStore( "/x" ), 14);
function suppliedStillWins( required string directory, any shadowSvc ) { arguments.directory = "z"; return shadowSvc; }
assert("a supplied value still wins over variables", suppliedStillWins( "/x", "SUPPLIED" ), "SUPPLIED");

suiteEnd();
</cfscript>
