<cfscript>
suiteBegin("Core: component declaration-header attribute parsing");

// ============================================================
// Background
// ============================================================
// CFML component declarations carry zero or more metadata attributes in the
// header before the body brace, e.g. `component output="false" extends="Foo" {`.
// On Lucee 5/6/7, Adobe ColdFusion 2018-2025, and BoxLang these attributes are
// ORDER-INDEPENDENT and their values may be written quoted OR unquoted (a bare
// boolean keyword / identifier is a legal attribute value).
//
// RustCFML 0.36.0 made `component` a soft keyword and accepts a single leading
// metadata attribute, but two header shapes that the framework relies on still
// fail to parse — and because an unparseable component degrades to a non-object
// (silently, no throw), the failure is invisible until you ask `isObject()`:
//
//   A. `extends` is only accepted as the FIRST attribute. Placed after another
//      attribute it fails: `Parse error: Expected LBrace, found Extends`.
//        component extends="Base" output="false" {}   -> parses (control)
//        component output="false" extends="Base" {}   -> FAILS  (gap A)
//      This is the dominant Wheels header shape — the entire boot cascade is
//      `component output="false" ... extends="wheels.Global" {`.
//
//   B. An UNQUOTED boolean attribute value is rejected. The value `false` lexes
//      as a Boolean-literal token the attribute parser will not accept:
//        component output="false" {}   -> parses (control)
//        component output=false {}     -> FAILS  (gap B: Expected LBrace, found False)
//      Wheels writes its database adapters this way:
//        component extends="wheels.databaseAdapters.Base" output=false {}
//
// The failing headers live in runtime-instantiated FIXTURE CFCs (not inline)
// because a parse error escapes try/catch and would abort the whole runner; via
// createObject the unparseable fixture degrades to a non-object instead.
// ============================================================

// Load a fixture and return its ping(); a sentinel if the header failed to parse.
// On every JVM engine the fixture parses and ping() returns "pong". When the
// header fails to parse on RustCFML, createObject yields a non-object, so we
// surface "NOT-A-COMPONENT" and the assertion shows the gap.
function loadPing(required string name) {
	var o = createObject("component", arguments.name);
	return isObject(o) ? o.ping() : "NOT-A-COMPONENT";
}

// --- controls: header shapes RustCFML already accepts (regression guards) ----

assert("control: `extends` as the FIRST attribute parses", loadPing("ExtendsFirstFixture"), "pong");
assert("control: a quoted boolean attribute value parses", loadPing("QuotedBoolFixture"), "pong");

// --- gap A: `extends` after another attribute --------------------------------

assert("`extends` after another attribute parses (output=... extends=...)",
	loadPing("ExtendsAfterAttrFixture"), "pong");

// --- gap B: unquoted boolean attribute value ---------------------------------

assert("an unquoted boolean attribute value parses (output=false)",
	loadPing("UnquotedBoolFixture"), "pong");

suiteEnd();
</cfscript>
