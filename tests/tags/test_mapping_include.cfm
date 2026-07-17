<cfscript>
suiteBegin("Includes: this.mappings-based include path resolution");

// ============================================================
// Background
// ============================================================
// A CFML application registers path mappings via `this.mappings` in
// Application.cfc. On Lucee 5/6/7, Adobe CF 2018-2025, and BoxLang those
// mappings resolve BOTH component paths (createObject/new) AND template paths
// used by cfinclude.
//
// tests/Application.cfc maps "/wheelsmapprobe" to the tests/tags/ directory.
// That mapping name is deliberately NOT a real webroot subdirectory, so a
// `/wheelsmapprobe/...` include can ONLY be resolved via the mapping --
// webroot-relative resolution cannot find it. (Using a name like "/tags" would
// be ambiguous: tests/tags/ exists under the webroot, so a leading-slash
// include would resolve webroot-relative even without consulting the mapping.)
// This isolates this.mappings resolution as the behavior under test.
//
// RustCFML resolves the COMPONENT form of mappings (createObject("component",
// "wheels.Injector"), new wheels.X all work) but does NOT apply mappings to
// cfinclude template paths: it reads the LITERAL path from the filesystem root
// ("Cannot read '/wheelsmapprobe/mapped_include_target.cfm': No such file or
// directory"). NB: that read-failure is also NOT catchable via try/catch on
// RustCFML, so this test exercises the include inside a fixture's
// PSEUDO-CONSTRUCTOR -- there the failure degrades the component to a
// non-object silently (the same way wheels.Global degrades), which keeps the
// run going and lets the assertion below fail cleanly rather than aborting.
//
// Why it matters for Wheels: public/.../Global.cfc's pseudo-constructor runs
//
//     include "/app/global/functions.cfm";
//
// relying on the "/app" mapping (app/ is a sibling of the public/ webroot, so
// it is reachable ONLY via the mapping). On RustCFML that include reads the
// literal path, throws, and -- because the failing pseudo-constructor degrades
// wheels.Global to a non-object silently -- the DI Injector's
// getInstance("global") returns a non-object, application.wo no-ops, and the
// request renders empty. This is the runtime blocker that remains after the
// parse-level gaps on the boot path are cleared.
//
// This assertion PASSES on Lucee/ACF/BoxLang.
// ============================================================

ok = false;
probe = "";
try {
	probe = createObject("component", "MappingIncludeFixture");
	ok = isObject(probe);
} catch (any e) {
	ok = false;
}

assert("include of a this.mappings path (/wheelsmapprobe/...) resolves and runs at instantiation",
	ok ? probe.getMarker() : "(non-object: pseudo-constructor include failed)", "MAPPED_INCLUDE_OK");

// ============================================================
// Leading multi-slash includes collapse to a single '/'.
// ============================================================
// Lucee/ACF/BoxLang (and expandPath) treat `include "//x"` / `"///x"` the same
// as `"/x"`. ColdBox's FrameworkSupertype.includeUDF builds its include path as
// `"/" & appMapping & "/" & udflibrary` with an EMPTY appMapping and an
// already-rooted udflibrary ("/coldbox/…"), yielding "///coldbox/…"; the extra
// leading slashes must NOT defeat this.mappings resolution. Regression for the
// ColdBox-boot include fix.
request.mappedIncludeMarker = "";
include "//wheelsmapprobe/mapped_include_target.cfm";
assert("double-slash include collapses and resolves via the mapping",
	request.mappedIncludeMarker, "MAPPED_INCLUDE_OK");

request.mappedIncludeMarker = "";
include "///wheelsmapprobe/mapped_include_target.cfm";
assert("triple-slash include collapses and resolves via the mapping",
	request.mappedIncludeMarker, "MAPPED_INCLUDE_OK");

suiteEnd();
</cfscript>
