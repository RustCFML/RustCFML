<cfscript>
suiteBegin("return of an assignment expression (GH ##259 cascade)");

// `return x = expr` assigns AND yields the assigned value (Lucee/ACF/BoxLang).
// RustCFML previously stored the value but left nothing on the stack, so the
// function returned null. Preside's SystemAlertsService.getCriticalAlerts does
// `return alerts = obj.selectData(...)`; the null return then made the caller's
// `criticalAlerts.recordCount` throw "variable is undefined" — the blocker that
// surfaced on the admin sitetree once the GH #259 renderViewlet fix landed.

simple = function() { return x = 42; };
assert( "return of a simple assignment yields the value", simple(), 42 );

arr = function() { return y = [ 1, 2, 3 ]; };
r = arr();
assertTrue( "return of an array assignment yields the array", isArray( r ) && arrayLen( r ) == 3 );

// The assignment must also have taken effect in the function's own scope
// (return-assign is an assignment, not just an expression).
memberFn = function() {
	var s = {};
	s.total = ( s.count = 5 );   // nested assignment value in a member target
	return outVal = s.count + s.total;
};
assert( "nested assignment value threads through", memberFn(), 10 );

// Compound-free chained form: return the last of a chain.
chain = function() { var a = 0; return a = ( innerB = 9 ); };
assert( "chained return-assignment yields the innermost value", chain(), 9 );

suiteEnd();
</cfscript>
