<cfscript>
// A callee must NOT see the CALLER's function-LOCAL variables. CFML is
// lexically (not dynamically) scoped: a bare read of a name that is undefined
// in the callee's own scopes resolves to variables/globals or null — never the
// caller's `local` scope. Exposed by the `x ?: default` idiom, which must yield
// the default when x is genuinely undefined, not the caller's same-named local.
//
// Repro family: Preside SiteTreeService._createSystemPage — a private method
// read `parent ?: ""` where `parent` was only ever assigned inside a not-taken
// branch of a DIFFERENT (recursive) invocation; the caller's leaked local (an
// empty query) reached the callee and broke fresh-DB site-tree creation with
// "SQL Param values must be simple values". Lucee-verified.
suiteBegin("Caller-local scope isolation");

// --- Named sibling UDFs: callee must not see caller's var-local ---------------
function leakSecret() {
	var secret = "LEAKED";
	return leakReader();
}
function leakReader() {
	return secret ?: "CLEAN";
}
assert("sibling UDF: caller local not visible", leakSecret(), "CLEAN");

// --- Recursion: inner call must not inherit outer call's branch-local ---------
function recurse( depth ) {
	if ( arguments.depth != "leaf" ) {
		var branchLocal = "OUTER-ONLY";
		return recurse( "leaf" );
	}
	// reached only on the recursive "leaf" call, where branchLocal was never
	// assigned in THIS invocation:
	return branchLocal ?: "CLEAN";
}
assert("recursion: outer branch-local not visible", recurse( "root" ), "CLEAN");

// --- Caller's param must not leak either ---------------------------------------
function paramHolder( p ) { return paramReader(); }
function paramReader() { return p ?: "CLEAN"; }
assert("caller param not visible", paramHolder( "PARAM-VAL" ), "CLEAN");

// --- Closures STILL capture their enclosing scope (incl. mutated data) --------
function makeAccumulator() {
	var total = 0;
	var walk  = function( n ) {
		total += n;
		if ( n > 1 ) { walk( n - 1 ); }
		return total;
	};
	return walk( 4 );
}
assert("closure captures + mutates enclosing var across recursion", makeAccumulator(), 10);

suiteEnd();
</cfscript>

<cfscript>
// Component-scope isolation lives in a CFC so `this`/`variables` are real.
suiteBegin("Caller-local scope isolation (component)");
leakCfc = new core.CallerLocalLeakCfc();
assert("method-to-method: this reachable, caller local not", leakCfc.run(), "this=ok;var=SHARED;leak=CLEAN");
assert("inherited page/component var propagates through call chain", leakCfc.chain(), "/cfg/path");
suiteEnd();
</cfscript>
