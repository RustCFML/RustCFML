<cfscript>
suiteBegin( "The page variables scope is one shared struct, not a copy per call" );
// A page's `variables` scope is a single struct handle. Functions and closures
// called from the page resolve page variables through that handle instead of
// receiving a copy of every page variable in their frame (and diffing it back
// on return), so a call's cost no longer grows with the number of page
// variables — and the scope is LIVE everywhere it is visible.
pageA = 1;
function readsPage() { return pageA; }
function writesPage() { pageB = "set-in-udf"; pageA = pageA + 1; }
assert( "UDF reads a page variable", readsPage(), 1 );
writesPage();
assert( "UDF classic-mode bare write creates a page variable", variables.pageB, "set-in-udf" );
assert( "UDF classic-mode bare write updates a page variable", pageA, 2 );
assert( "structKeyExists(variables) sees the UDF's write", structKeyExists( variables, "pageB" ), true );
// `variables` is a live handle, not a snapshot.
v = variables;
v.throughAlias = 7;
assert( "a `variables` reference is live: write via alias", throughAlias, 7 );
pageA = 3;
assert( "a `variables` reference is live: read via alias", v.pageA, 3 );
assert( "structCount(variables) counts page variables only, not builtins", structCount( variables ) < 50, true );
assert( "structKeyList(variables) excludes builtin functions", listFindNoCase( structKeyList( variables ), "arrayLen" ), 0 );
// Dynamic stores land in the page scope.
setVariable( "variables.dynA", 11 );
setVariable( "dynB", 12 );
assert( "setVariable('variables.x') lands in the page scope", variables.dynA, 11 );
assert( "setVariable('x') lands in the page scope", variables.dynB, 12 );
param name="variables.paramA" default="pa";
paramName = "paramB";
param name="#paramName#" default="pb";
assert( "param with a scoped name lands in the page scope", variables.paramA, "pa" );
assert( "param with a runtime name lands in the page scope", variables.paramB, "pb" );
// Closures defined on the page see the live page scope, not a snapshot.
pageC = "before";
cl = function() { return pageC; };
pageC = "after";
assert( "page closure reads the current page variable", cl(), "after" );
clw = function() { pageD = "from-closure"; };
clw();
assert( "page closure classic-mode bare write lands in the page scope", variables.pageD, "from-closure" );
// A UDF that defines a closure hands the page scope on.
function makeReader() { return function() { return pageA; }; }
r = makeReader();
pageA = 4;
assert( "closure made inside a UDF sees the live page scope", r(), 4 );
// Many page variables: a call still resolves the right one, and the loop
// variable of the page is untouched by the callee's own loop.
for ( n = 1; n <= 300; n++ ) variables[ "pv" & n ] = n;
function sumTwo( a, b ) { return a + b; }
total = 0;
for ( i = 1; i <= 300; i++ ) total += sumTwo( variables[ "pv" & i ], 0 );
assert( "300 page variables: every call resolves the right one", total, 45150 );
assert( "page loop variable untouched by the calls", i, 301 );
// Nested UDF calls: the scope is shared down the chain, not re-copied.
function outerFn() { return innerFn(); }
function innerFn() { pageE = "inner"; return pageA; }
assert( "nested UDF call reads the page scope", outerFn(), 4 );
assert( "nested UDF call writes the page scope", pageE, "inner" );
// A UDF's `variables.x` read/write is the page scope.
function scopedRW() { variables.pageF = variables.pageA * 10; return variables.pageF; }
assert( "variables.x inside a UDF called from a page is the page scope", scopedRW(), 40 );
assert( "and the write is visible on the page", pageF, 40 );
// An include shares the page scope in both directions.
include "pagescope_include.cfm";
assert( "include sees the page variable", incSaw, 4 );
assert( "include's new variable is a page variable", variables.fromInclude, "yes" );
assert( "include's write to an existing page variable lands", pageA, 5 );
suiteEnd();
</cfscript>
