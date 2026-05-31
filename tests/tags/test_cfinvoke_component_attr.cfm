<cfscript>
suiteBegin("Tags: cfinvoke with a `component` attribute (script statement form)");

// ============================================================
// Background
// ============================================================
// `component` is a SOFT keyword on Lucee 5/6/7, Adobe ColdFusion 2018-2025,
// and BoxLang: it introduces a CFC when it begins a declaration, but it is
// otherwise a normal identifier — usable as a variable name AND as the
// `component` attribute of <cfinvoke> / the cfscript `cfinvoke` statement:
//
//     cfinvoke component="MyService" method="doThing" returnVariable="r";
//
// RustCFML treats `component` as a HARD reserved keyword. Any place the token
// `component` is followed by `=` — a bare `component = x` assignment, or a
// `component = ...` attribute in a script-statement tag — fails to PARSE with
// "Expected LBrace, found Equal" (the parser commits to a `component { ... }`
// declaration and then hits the `=`). The function-CALL form
// `cfinvoke(component=...)` parses fine; only the statement form with a
// `component` attribute fails.
//
// Why it matters for Wheels: public/... Global.cfc::$cfinvoke() is written as
//
//     cfinvoke
//     component = "#arguments.component#"
//     method = "#arguments.method#"
//     returnVariable = "#arguments.returnVariable#"
//     argumentCollection = "#arguments.invokeArguments#";
//
// so wheels.Global fails to parse. Because RustCFML degrades an unparseable
// component to a non-object SILENTLY (no thrown error),
// createObject("component","wheels.Global") returns a non-object, the Wheels
// DI Injector's getInstance("global") hands that non-object to application.wo,
// and every framework lifecycle call on application.wo (onApplicationStart ->
// $cgiScope; onRequestStart -> $initializeRequestScope / $runOnRequestStart)
// silently no-ops. The framework "boots" and then serves empty 200 responses
// with no error. This single parser gap is the terminal blocker to serving a
// Wheels request on RustCFML.
//
// The parse failure is CONTAINED in a runtime-instantiated fixture
// (CfInvokeComponentAttrFixture) so it degrades that component to a non-object
// silently instead of aborting this run. All assertions PASS on
// Lucee/ACF/BoxLang.
// ============================================================

result = "(start)";
try {
	probe = createObject("component", "CfInvokeComponentAttrFixture");
	if (isObject(probe)) {
		result = probe.callViaCfinvoke();
		if (isNull(result)) {
			result = "(null - method no-op'd)";
		}
	} else {
		// RustCFML: the fixture failed to parse -> non-object.
		result = "(non-object - component failed to parse)";
	}
} catch (any e) {
	result = "(threw: " & e.message & ")";
}

assert("cfinvoke component=... parses; the fixture instantiates and invokes its method",
	result, "INVOKED_OK");

suiteEnd();
</cfscript>
