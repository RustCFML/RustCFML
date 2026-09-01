<cfscript>
// Classic CF debug-footer BIFs (Phase 1 of the observability plan).
//
// These BIFs (isDebugMode / getDebugData / debugAdd) are RustCFML/Lucee-style
// debug surface, so the assertions are guarded to RustCFML — Lucee does not
// expose this exact trio. tests/.cfconfig.json sets `debugging.enabled = true`
// and the CLI runner is loopback, so the footer's activation gates pass and the
// per-request collector IS active here (the HTML panel itself is web-only and
// not appended to CLI stdout — see maybe_render_debug_footer). Footer rendering
// + the IP/URL-trigger gates are covered by the Rust gate tests in
// crates/cfml-vm/src/lib.rs (`debug_footer_gate_tests`).
suiteBegin("Debug footer BIFs");

if (isRustCFML() && isDebugMode()) {
    // isDebugMode(): boolean. True when debugging is enabled (tests/.cfconfig.json
    // in the CLI runner) AND the request comes from a whitelisted (loopback) IP,
    // so the per-request collector is installed. When the footer ISN'T active —
    // e.g. serving from a webroot whose .cfconfig doesn't enable debugging —
    // getDebugData() is an empty struct by design and these shape assertions
    // don't apply, so the block is skipped (see the else branch). Footer gates
    // themselves are covered by the Rust `debug_footer_gate_tests`.
    var dm = isDebugMode();
    assert("isDebugMode returns boolean", isBoolean(dm), true);

    // getDebugData(): a struct with the Lucee-shaped sections.
    var dd = getDebugData();
    assert("getDebugData returns a struct", isStruct(dd), true);
    assert("getDebugData has queries array", isArray(dd.queries), true);
    assert("getDebugData has genericData array", isArray(dd.genericData), true);
    assert("getDebugData has total", structKeyExists(dd, "total"), true);

    // debugAdd(): the genericData channel. A row added must surface in the next
    // getDebugData() read.
    var before = arrayLen(getDebugData().genericData);
    debugAdd("DebugFooterTest", "marker", "hello");
    var after = getDebugData().genericData;
    assert("debugAdd appended a genericData row", arrayLen(after) == before + 1, true);
    var last = after[arrayLen(after)];
    assert("debugAdd row category", last.category, "DebugFooterTest");
    assert("debugAdd row name", last.name, "marker");
    assert("debugAdd row value", last.value, "hello");

    // debugAdd(category, struct) form: one row per struct key.
    var b2 = arrayLen(getDebugData().genericData);
    debugAdd("DebugFooterTest", { "k1": "v1", "k2": "v2" });
    assert("debugAdd struct form added two rows",
        arrayLen(getDebugData().genericData) == b2 + 2, true);

    // A CAUGHT exception still feeds the Exceptions section (Lucee parity —
    // recorded at the throw site, not only on uncaught propagation).
    var exBefore = arrayLen(getDebugData().exceptions);
    try {
        throw(type="DebugFooterTest.Boom", message="caught on purpose");
    } catch (any e) {}
    var exAfter = getDebugData().exceptions;
    assert("caught exception recorded", arrayLen(exAfter) == exBefore + 1, true);
    assert("exception type captured", exAfter[arrayLen(exAfter)].type, "DebugFooterTest.Boom");

    // writeLog + trace feed the traces section.
    var trBefore = arrayLen(getDebugData().traces);
    writeLog(text="footer test log", type="information", file="debugfootertest");
    trace("footer test trace");
    assert("writeLog + trace recorded as traces",
        arrayLen(getDebugData().traces) == trBefore + 2, true);

    // Regression (v0.524): a CFC method call must feed the `pages` (templates)
    // section. The v0.519 flyweight flip made components `Instance` values, but
    // both template-timing hooks only fired for marker `Struct` receivers, so
    // every CFC method call recorded ZERO page rows — the debug footer collapsed
    // to just the .cfm includes + Application.cfc lifecycle. Instantiating a
    // component that EXTENDS another and calling its method must now surface a
    // `pages` row whose id resolves to DebugFooterKid.cfc.
    var kid = new observe.DebugFooterKid();
    assert("inherited method runs", kid.kidRun(), "hello from base via kid");
    var pageIds = getDebugData().pages.map((p) => lCase(p.id));
    var kidRecorded = pageIds.some((id) => id.findNoCase("debugfooterkid.cfc") > 0);
    assert("CFC method call recorded a pages row (flyweight regression guard)",
        kidRecorded, true);

    // Regression (v0.645): CFC CONSTRUCTION must feed the `pages` section too.
    // Only component METHODS opened a timed frame, so `new X()` was invisible —
    // and construction is not cheap: 200 `new` of a 40-method CFC measured
    // 7.4ms with no row of its own anywhere in the footer, all of it dumped into
    // the top-level page's residual. This fixture is constructed and NEVER
    // method-called, so a row for it can only come from the construction.
    var ctorOnly = new observe.DebugFooterCtorOnly();
    assert("ctor-only fixture constructed", isObject(ctorOnly), true);
    var ctorRows = getDebugData().pages.filter(
        (p) => lCase(p.id).findNoCase("debugfooterctoronly.cfc") > 0
    );
    assert("construction alone recorded a pages row", arrayLen(ctorRows) > 0, true);
    // ...and it is labelled as the constructor, not as a method, so it can never
    // silently merge with a real method of the same name.
    var ctorMethods = ctorRows[1].methods.map((m) => m.name);
    assert("construction row is labelled <constructor>",
        ctorMethods.some((n) => n == "<constructor>"), true);

    // A `<cfmodule>` / custom-tag execution must ALSO feed the `pages` section.
    // Lucee's Execution Time section is documented as covering "templates,
    // includes, modules, custom tags, and component method calls"; RustCFML
    // instrumented includes, CFC methods and Application.cfc lifecycle but not
    // the custom-tag/cfmodule path, so tag-heavy apps (Preside renders every
    // view through `module attributeCollection=…`) showed no row for them.
    savecontent variable="modOut" {
        module template="debug_footer_module.cfm";
    }
    assert("module fixture ran", trim(modOut), "[module ran]");
    var modIds = getDebugData().pages.map((p) => lCase(p.id));
    assert("cfmodule/custom-tag execution recorded a pages row",
        modIds.some((id) => id.findNoCase("debug_footer_module.cfm") > 0), true);
} else if (isRustCFML()) {
    // Debug footer not active for this request (debugging disabled in the served
    // webroot's .cfconfig, or a non-whitelisted viewer). getDebugData() is empty
    // by design — record an informational pass rather than erroring on the
    // absent sections. The CLI runner (tests/.cfconfig enables debugging)
    // exercises every assertion above.
    assert("isDebugMode returns boolean", isBoolean(isDebugMode()), true);
    assert("debug footer inactive here — shape assertions skipped", true, true);
}

suiteEnd();
</cfscript>
