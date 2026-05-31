<cfscript>
suiteBegin("Core: reserved words usable as identifiers");

// ============================================================
// Background
// ============================================================
// Several words RustCFML treats as hard reserved keywords are only SOFT
// keywords on Lucee 5/6/7, Adobe ColdFusion 2018-2025, and BoxLang — reserved
// for one grammatical role, but otherwise legal as ordinary identifiers. v0.36.0
// already made `component` soft (usable as a variable / struct key / cfinvoke
// attribute). Two more that the Wheels framework relies on are still hard:
//
//   * `new` as a FUNCTION NAME. `new Foo()` is the instantiation operator, but
//     `new` is also a valid method name. Wheels' core creation API is built on
//     it: `model("User").new()` -> `public any function new(...)` in
//     vendor/wheels/model/create.cfc. On RustCFML the declaration
//     `function new(){...}` fails to parse: "Expected identifier, found New".
//
//   * `extends` / `implements` as PARAMETER NAMES. Both are declaration keywords
//     but legal argument names. Wheels uses them in
//     vendor/wheels/wheelstest/system/mockutils/MockGenerator.cfc:
//     `function generateClass( string extends="", string implements="" )`.
//     On RustCFML the parameter declaration fails: "Expected RParen, found Extends".
//
// An unparseable component degrades to a non-object (silently, no throw), so the
// failing declarations live in fixtures and are reached via createObject; the
// helper returns a sentinel when the fixture did not parse.
// ============================================================

function loadProbe(required string name) {
	var o = createObject("component", arguments.name);
	return isObject(o) ? o.probe() : "NOT-A-COMPONENT";
}

assert("a method named `new` parses and is callable (model().new())", loadProbe("NewMethodFixture"), "made");
assert("`extends`/`implements` are usable as parameter names", loadProbe("ReservedParamFixture"), "a/b");

suiteEnd();
</cfscript>
