<cfscript>
suiteBegin( "arguments scope dispatch (call-dispatch Lever 2 — __arguments_params memo/skip)" );

// These lock in the behaviours that depend on the internal `__arguments_params`
// positional marker, which the call-dispatch optimisation now (a) omits entirely
// for paramless functions and (b) memoises+shares for functions with params.
// All must pass identically on Lucee (cross-engine oracle).

// --- positional arguments[N] on a function WITH declared params, called by name ---
function withParams( a, b, c ) {
    return arguments[ 1 ] & "|" & arguments[ 2 ] & "|" & arguments[ 3 ];
}
assert( "positional access, named call", withParams( a="x", b="y", c="z" ), "x|y|z" );
assert( "positional access, positional call", withParams( "p", "q", "r" ), "p|q|r" );

// --- reading an omitted optional param yields null (not "undefined" throw) ---
function optionalArg( required a, b ) {
    return isNull( arguments.b ) ? "B_IS_NULL" : arguments.b;
}
assert( "omitted optional param reads null", optionalArg( "only-a" ), "B_IS_NULL" );

// --- paramless function called positionally: overflow numeric keys, [N] works ---
// (this is the empty-params fast path: __arguments_params is omitted, GetIndex
//  falls through to the N-th non-marker entry)
paramless = function() {
    return arguments[ 1 ] & "," & arguments[ 2 ] & " count=" & arrayLen( arguments );
};
assert( "paramless positional access", paramless( "one", "two" ), "one,two count=2" );

// --- markers are hidden from all struct introspection on the arguments scope ---
function introspect( x, y ) {
    return {
          keyExistsMarker = structKeyExists( arguments, "__arguments_params" )
        , keyList         = listSort( structKeyList( arguments ), "textnocase" )
        , count           = structCount( arguments )
        , forInKeys       = ""
    };
}
r = introspect( 1, 2 );
assertFalse( "structKeyExists hides __arguments_params", r.keyExistsMarker );
assert( "structKeyList excludes markers", r.keyList, "x,y" );
assert( "structCount excludes markers", r.count, 2 );

// for-in over arguments excludes markers
function forInArgs( m, n ) {
    keys = [];
    for ( k in arguments ) { keys.append( k ); }
    return listSort( arrayToList( keys ), "textnocase" );
}
assert( "for-in over arguments excludes markers", forInArgs( 10, 20 ), "m,n" );

// --- argumentCollection still forwards correctly (arguments as a struct) ---
function target( one, two, three ) {
    return arguments.one & arguments.two & arguments.three;
}
function forwarder( one, two, three ) {
    return target( argumentCollection = arguments );
}
assert( "argumentCollection forwards named", forwarder( one="A", two="B", three="C" ), "ABC" );

// --- memo correctness: same function, many calls, distinct arg values ---
// (shared __arguments_params Arc must never leak values between invocations)
function echo3( p, q, r ) { return arguments[1] & arguments[2] & arguments[3]; }
ok = true;
for ( i = 1; i <= 50; i++ ) {
    if ( echo3( i, i*2, i*3 ) != ( i & (i*2) & (i*3) ) ) { ok = false; break; }
}
assertTrue( "repeated calls keep per-call arg values (no shared-marker leak)", ok );

// --- bare (unscoped) read of an UNDECLARED arg resolves via the arguments scope ---
// An extra named arg — passed positionally-by-name or forwarded via
// argumentCollection — that does not bind to a formal parameter lives only in the
// arguments scope. A bare (unscoped) reference to it must still resolve, because
// CFML searches local -> arguments -> variables. RustCFML previously copied only
// declared params into the bare-name search space, so `bareUndeclared` below
// silently read as missing.
function bareUndeclared( declared ) {
    return ( extra ?: "MISSING" );  // `extra` never declared as a param
}
assert( "bare read of undeclared named arg", bareUndeclared( declared="d", extra="hello" ), "hello" );

// case-insensitive bare read of an undeclared arg
function bareUndeclaredCI( declared ) {
    return ( EXTRA ?: "MISSING" );
}
assert( "bare read of undeclared named arg (ci)", bareUndeclaredCI( declared="d", extra="world" ), "world" );

// --- ColdBox preHandler pattern: extra args forwarded via argumentCollection to
// a callee that doesn't declare them, then read as bare names. This is exactly
// how Preside's DataManager preHandler passes rc/prc into _loadTopRightButtons,
// whose bare `prc.objectName` fed hasWorkflow() (regression: "Object [] does not
// exist" on admin.datamanager.viewRecord).
function innerHelper( event, action ) {
    return ( prc.objectName ?: "" );  // prc is undeclared here
}
function outerHandler( event, action ) {
    return innerHelper( argumentCollection = arguments );  // forwards rc/prc too
}
assert(
      "argumentCollection-forwarded undeclared arg readable as bare name in callee"
    , outerHandler( event="e", action="viewRecord", rc={ object="x" }, prc={ objectName="content_library_content" } )
    , "content_library_content"
);

// --- precedence: a real local shadows a same-named undeclared arg ---
function localBeatsArg( declared ) {
    var extra = "LOCAL_WINS";
    return extra;  // must be the local, not the undeclared arg
}
assert( "local var outranks same-named undeclared arg", localBeatsArg( declared="d", extra="ARG" ), "LOCAL_WINS" );

suiteEnd();
</cfscript>
