<cfscript>
suiteBegin("Core: reserved words usable as identifiers");

// ============================================================
// Background  (adapted from PR #32 by bpamiri)
// ============================================================
// Several words RustCFML treats as reserved keywords are only SOFT keywords on
// Lucee 5/6/7, Adobe ColdFusion 2018-2025, and BoxLang — reserved for one
// grammatical role, but otherwise legal as ordinary identifiers. v0.36.0 made
// `component` soft; PR #30 made keyword method names like `new` legal. Two more
// the Wheels framework relies on:
//
//   * `new` as a FUNCTION NAME. `new Foo()` is the instantiation operator, but
//     `new` is also a valid method name (model("User").new() ->
//     public any function new(...) in vendor/wheels/model/create.cfc).
//
//   * `extends` / `implements` as PARAMETER NAMES. Both are declaration keywords
//     but legal argument names — used in
//     vendor/wheels/wheelstest/system/mockutils/MockGenerator.cfc:
//     `function generateClass( string extends="", string implements="" )`.
// ============================================================

function loadProbe(required string name) {
	o = createObject("component", arguments.name);
	return isObject(o) ? o.probe() : "NOT-A-COMPONENT";
}

assert("a method named `new` parses and is callable (model().new())", loadProbe("NewMethodFixture"), "made");
assert("`extends`/`implements` are usable as parameter names (arguments scope)", loadProbe("ReservedParamFixture"), "a/b");

// `extends` / `implements` also resolve as bare identifiers in expressions,
// not only through the arguments scope.
bareNames = function(string extends = "", string implements = "") {
	return extends & "/" & implements;
};
assert("`extends`/`implements` resolve as bare parameter references", bareNames("x", "y"), "x/y");

// `imp` is legal as an ordinary local variable name on Lucee. It appears in
// short-lived impersonation-context locals (for example, `var imp = ...`) and
// must not be tokenized as a hard keyword in expression contexts.
function impIdentifierProbe() {
	var imp = "ok";
	return imp;
}

assert("`imp` is usable as a local variable name", impIdentifierProbe(), "ok");

suiteEnd();
</cfscript>
