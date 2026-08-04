<!---
  A page-scope variable whose VALUE is a function must be readable from inside
  another function body — bare, scope-qualified, and from a sibling closure.
  Regression cover for docs/known-issues.md §32: function-expression page vars
  were dropped on the way into a callee frame (the parent-scope seed skipped
  every `CfmlValue::Function` carrying a captured scope, on the assumption it
  was reachable via `user_functions` — true only for DECLARED functions), so
  `cl( "x" )` inside any function threw "Variable 'cl' is undefined".

  Every assertion here was verified against Lucee 7.
--->
<cfscript>
suiteBegin( "Page-scope function variables (§32)" );

// --- the three shapes that used to throw -------------------------------------
cl = function( x ) { return "called:" & x; };

fromClosure = function() { return cl( "v" ); };
function fromNamed()  { return cl( "n" ); }
function fromScoped() { return variables.cl( "s" ); }

assert( "bare read from a closure",        fromClosure(), "called:v" );
assert( "bare read from a named function", fromNamed(),   "called:n" );
assert( "variables-qualified read",        fromScoped(),  "called:s" );

// A plain (non-function) page var was always fine — pin it so a future fix
// can't regress the ordinary case while chasing the function one.
pv = "plain";
function readsPlain() { return pv; }
assert( "plain page var still readable", readsPlain(), "plain" );

// --- declaration order ------------------------------------------------------
// The read happens at CALL time, so a function may reference a page-level
// function var declared BELOW it.
function usesLater() { return later( 3 ); }
later = function( n ) { return n * 2; };
assert( "callee declared after the caller", usesLater(), 6 );

// --- a DECLARED function assigned to a page var -----------------------------
// `helper` is in the engine's function table; `aliasOfHelper` is not, so the
// alias has to travel as a value.
function helper( s ) { return "helped:" & s; }
aliasOfHelper = helper;
function viaAlias() { return aliasOfHelper( "a" ); }
assert( "alias of a declared function", viaAlias(), "helped:a" );

// --- nesting ----------------------------------------------------------------
// Two frames deep: the page var must survive more than one hop.
function inner() { return cl( "deep" ); }
function outerHop() { return inner(); }
assert( "readable two frames down", outerHop(), "called:deep" );

// A page function var called from inside a closure that is itself called from
// a named function.
function callsAClosure() {
    localClosure = function() { return cl( "nested" ); };
    return localClosure();
}
assert( "read from a closure made inside a function", callsAClosure(), "called:nested" );

// --- the page var stays a function, and stays callable at page level --------
assert( "isCustomFunction on the page var", isCustomFunction( cl ), true );
assert( "still callable at page scope", cl( "top" ), "called:top" );

// --- containers -------------------------------------------------------------
// A function reached through a struct/array held in a page var.
bag = { doubler = function( n ) { return n * 2; } };
list = [ function( n ) { return n + 1; } ];
function viaStruct() { return bag.doubler( 21 ); }
// Via a local, not `list[ 1 ]( 41 )` — Lucee 7 rejects an immediate call on a
// subscript ("The function [1] does not exist in the Array").
function viaArray()  { var f = list[ 1 ]; return f( 41 ); }
assert( "function inside a page-scope struct", viaStruct(), 42 );
assert( "function inside a page-scope array",  viaArray(),  42 );

// --- a caller's OWN local must NOT leak into a callee -----------------------
// The fix carries page-scope function vars; it must not turn on dynamic
// scoping for a function-valued var declared local to the CALLER.
function declaresLocalFn() {
    var privateFn = function() { return "private"; };
    return readsCallersLocal();
}
function readsCallersLocal() {
    return isDefined( "privateFn" ) ? "leaked" : "not-visible";
}
assert( "caller's local function var does not leak", declaresLocalFn(), "not-visible" );

suiteEnd();
</cfscript>
