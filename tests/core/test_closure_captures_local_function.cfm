<cfscript>
suiteBegin("Closure captures enclosing var-scoped function");

// ============================================================
// Gap surfaced laddering the Wheels framework test suite on RustCFML 0.273.0
// ============================================================
// A closure captures the var-scoped variables of its enclosing function. On
// Lucee/ACF/BoxLang that includes var-scoped FUNCTION EXPRESSIONS: a helper
// declared `var fn = function(){...}` in the outer function can be called by
// bare name from inside a nested closure. RustCFML captures plain var VALUES
// (strings, numbers, structs) correctly, but a var-scoped function expression
// is NOT visible inside the closure — a bare call throws
// "Variable 'fn' is undefined", and even isClosure(fn) / a bare read throws.
//
// This is the standard test-helper pattern: define a helper at the top of a
// spec's run() and call it from inside the it()/describe() closures. The Wheels
// core suite uses it widely (e.g. MigratorInfoSpec `var insertOrphan = ...`
// called from nested it() closures; packages specs `var $withEnv`, `$freshCache`).
// On RustCFML every such call errors.
//
// catch-body locals don't persist on every engine, so outcomes are recorded in
// a struct FIELD.
// ============================================================

function buildScope() {
	// var-scoped values AND a var-scoped function expression in the enclosing fn
	var strVal = "captured-string";
	var fnVal  = function (required string x) {
		return "fn:" & arguments.x;
	};
	var out = {};
	// a nested closure that reads both
	var inner = () => {
		try { out.str = strVal; }       catch (any e) { out.str = "ERR:" & e.message; }
		try { out.fn  = fnVal("ok"); }   catch (any e) { out.fn  = "ERR:" & e.message; }
	};
	inner();
	return out;
}

result = buildScope();

// CONTROL: plain var VALUE is captured by the nested closure (already works on RustCFML)
assert("nested closure captures an enclosing var-scoped string", result.str, "captured-string");

// THE GAP: nested closure must also see the enclosing var-scoped FUNCTION expression
assert("nested closure can call an enclosing var-scoped function", result.fn, "fn:ok");

suiteEnd();
</cfscript>
