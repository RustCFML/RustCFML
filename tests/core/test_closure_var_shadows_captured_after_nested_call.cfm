<cfscript>
suiteBegin( "A closure's var-locals survive a nested closure call when a same-named variable exists in the captured scope" );
// Regression: after any closure call inside a closure, the frame reconciled its
// captured env back into its locals — including keys the frame had declared
// with `var`. A `var i` (never forward-synced to the env) was overwritten by
// the enclosing scope's `i`, so with a page-level `i` left over from an
// earlier loop the closure's own `for (var i…)` ran once (page i = 5000) or
// never ended (page i = 201). A nested UDF call did not trigger it.
function viaParam( fn ) { return fn(); }
function runLoop( n ) {
	return viaParam( function() {
		var f = function( x ) { return x + 1; };
		var t = 0; var iters = 0;
		for ( var i = 1; i <= n; i++ ) { iters++; t += f( i ); if ( iters > n * 3 ) break; }
		return { t: t, iters: iters };
	} );
}
r = runLoop( 300 );
assert( "clean page: loop runs n times", r.iters, 300 );
i = 5000; t = 999999;
r = runLoop( 300 );
assert( "page i=5000 present: loop still runs n times", r.iters, 300 );
assert( "page t present: closure's var t is not clobbered", r.t, 45450 );
i = 201;
r = runLoop( 300 );
assert( "page i=201 present: loop terminates after exactly n iterations", r.iters, 300 );
assert( "page variables untouched by the closure's var-locals", i & "/" & t, "201/999999" );
// A var-local declared BEFORE the closure, then mutated inside it, is still
// reconciled even when the page has a same-named variable.
i = 5000;
function countWithSharedName() {
	return viaParam( function() {
		var i = 0;
		var bump = function() { i++; };
		for ( var k = 1; k <= 5; k++ ) bump();
		return i;
	} );
}
assert( "closure mutation of a var-local that shadows a page var still lands", countWithSharedName(), 5 );
assert( "page i untouched by that mutation", i, 5000 );
// The write-through a closure IS allowed to make (mutating an enclosing
// variable it did not declare) still works.
counter = 0;
bump = function() { counter++; };
viaParam( function() { for ( var k = 1; k <= 5; k++ ) bump(); } );
assert( "closure mutation of an enclosing variable still propagates", counter, 5 );
suiteEnd();
</cfscript>
