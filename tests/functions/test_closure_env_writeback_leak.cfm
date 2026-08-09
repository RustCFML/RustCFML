<cfscript>
// ============================================================
// Closure captured-env must NOT leak into the caller on return
//
// Regression test for the Preside sitetree "Variable 'page_type' is
// undefined" bug: the classic-localMode return writeback diffs the callee's
// locals against its parent scope. When a closure is dispatched as a VALUE
// (higher-order builtins like arrayEach route the callback through
// call_function), its captured env is seeded into its frame locals — and the
// writeback diff must treat those env entries as pre-existing (baseline),
// never as fresh writes to spray into the CALLER's frame. The fused
// call-parent merge (perf plan 3.2 stage 1) briefly lost that: after any
// arrayEach(arr, closure) call, the closure's captured env keys (args,
// permissions, mapping, ...) landed in the calling function's locals,
// shadowing same-named variables scope entries.
// ============================================================
suiteBegin("Closure env writeback leak");

function makeLeakyClosure() {
    var args      = { poison = 1 };
    var envmarker = "LEAKME";
    return function( x ) { var s = { a = 1 }; return structCount( s ); };
}

function callerFrame() {
    var cl = makeLeakyClosure();
    arrayEach( [ 1, 2 ], cl );
    // Neither of the closure's captured-env keys may appear in this frame.
    return {
        markerLeaked = isDefined( "envmarker" ),
        argsLeaked   = isDefined( "args" )
    };
}

r = callerFrame();
assertFalse( "closure env var does not leak into caller via arrayEach", r.markerLeaked );
assertFalse( "closure env 'args' does not leak into caller via arrayEach", r.argsLeaked );

// Same shape one level deeper: the caller reads a variables-scope struct
// bare, before and after the higher-order call — the closure's captured
// `args` must not shadow it (the Preside failure read args.page_type
// between two helper calls).
variables.args = { page_type = "good" };
function readsArgsAroundCall() {
    var cl = makeLeakyClosure();
    var before = args.page_type;
    arrayEach( [ 1 ], cl );
    var after = args.page_type; // threw "page_type is undefined" when leaked
    return before & "/" & after;
}
assert( "bare variables read unchanged across higher-order closure call", readsArgsAroundCall(), "good/good" );

suiteEnd();
</cfscript>
