<cfscript>
suiteBegin("Closure captures enclosing var-scoped function");

// ============================================================
// Gap surfaced laddering the Wheels framework test suite on RustCFML 0.273.0
// (PR #198). A closure captures the var-scoped variables of its enclosing
// function. On Lucee/ACF/BoxLang that includes var-scoped FUNCTION
// EXPRESSIONS: a helper declared `var fn = function(){...}` in the outer
// function can be called by bare name from inside a nested closure. RustCFML
// captured plain var VALUES (strings, numbers, structs) but NOT a var-scoped
// function expression — a bare call threw "Variable 'fn' is undefined".
//
// This is the standard test-helper pattern: define a helper at the top of a
// spec's run() and call it from inside the it()/describe() closures.
//
// catch-body locals don't persist on every engine, so outcomes are recorded
// in a struct FIELD.
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

// CONTROL: plain var VALUE is captured by the nested closure (already worked)
assert("nested closure captures an enclosing var-scoped string", result.str, "captured-string");

// THE GAP: nested closure must also see the enclosing var-scoped FUNCTION expression
assert("nested closure can call an enclosing var-scoped function", result.fn, "fn:ok");

suiteEnd();


suiteBegin("Closure captures enclosing var-scoped function (deferred / nested-closure)");

// ============================================================
// Second gap, surfaced laddering the Wheels suite on RustCFML 0.309.0. The
// single-level case above is now fixed, but the shape the Wheels core specs
// actually use is still broken: the capturing closure is DEFINED INSIDE an
// intermediate closure's body and INVOKED AFTER that intermediate has returned
// — i.e. describe(() => { it(() => { helper(); }); }), where TestBox stores the
// it() body and runs it later. RustCFML loses the grandparent var-scoped
// function expression across that intermediate boundary and throws
// "Variable 'fnVal' is undefined". Lucee/ACF/BoxLang keep the capture.
//
// This is the exact shape of MigratorInfoSpec / OrphanDetectionSpec /
// MigratorReconciliationSpec — `var insertOrphan = function(){...}` declared in
// run(), called from inside it() closures nested under describe() — which error
// "Variable 'insertOrphan' is undefined" on 0.309 (12 specs).
// ============================================================

function buildDeferredScope() {
	// var-scoped function expression in the ENCLOSING (grandparent) fn
	var fnVal = function (required string x) {
		return "fn:" & arguments.x;
	};
	var stored = [];
	// an intermediate closure that runs its argument synchronously (like describe())
	var intermediate = function (required any body) {
		arguments.body();
	};
	intermediate(() => {
		// inner closure DEFINED HERE (inside the intermediate's body), capturing the
		// grandparent fnVal, and STORED for later invocation (like it()'s body)
		arrayAppend(stored, () => {
			return fnVal("ok");
		});
	});
	var out = {};
	// pull the stored closure out before calling — arr[idx]() call syntax is rejected
	// by the Lucee/ACF parsers, so keep this cross-engine safe
	var deferred = stored[1];
	try { out.fn = deferred(); }          // invoke AFTER the intermediate returned
	catch (any e) { out.fn = "ERR:" & e.message; }
	return out;
}

deferredResult = buildDeferredScope();

// THE GAP: a closure defined inside an intermediate closure and invoked later must
// still see the enclosing var-scoped FUNCTION expression.
assert("deferred nested closure can call an enclosing var-scoped function", deferredResult.fn, "fn:ok");

suiteEnd();
</cfscript>
