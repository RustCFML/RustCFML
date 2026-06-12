<cfscript>
suiteBegin("Core: auto-vivify a nested write on a scope-QUALIFIED undeclared variable");

// ============================================================
// Background  (residual of test_undefined_var_autovivify.cfm)
// ============================================================
// On Lucee 5/6/7, Adobe ColdFusion 2018-2025, and BoxLang, assigning to a
// member path of an undeclared variable auto-creates the container as a
// struct even when the lvalue is SCOPE-QUALIFIED:
//
//     variables.$class.name = "x";   // variables.$class never initialized
//     request.ctx.user      = "x";   // request.ctx never initialized
//     local.cache.hit       = "x";   // (in a function) local.cache never initialized
//
// The engine implicitly performs `variables.$class = {}` on the first
// nested write, exactly as it does for the UNSCOPED form (x.a = 1).
//
// RustCFML fixed the unscoped form (test_undefined_var_autovivify.cfm) and
// the subscript form (test_subscript_autovivify.cfm); the scope-qualified
// nested write is the residual, and on 0.130.0 it breaks in two different
// ways:
//
//   - at template top level, `variables.X.name = "v"` THROWS
//     "Variable 'X' is undefined";
//   - everywhere else -- request./local. in any context, and variables.
//     inside a function or CFC method -- the write is LOST SILENTLY: the
//     scope key is registered (structKeyExists -> true) but bound to a
//     non-struct empty value. isStruct() -> false, the written member is
//     gone, and reading it back yields "" instead of throwing. No error
//     at the write, no error at the read; the data just vanishes.
//
// Wheels hits the exact CFC-method shape on the FIRST line of controller
// class initialization (vendor/wheels/Controller.cfc, $initControllerClass):
//
//     variables.$class.name = arguments.name;   // $class never pre-seeded
//     variables.$class.filters = [];
//     ...
//
// On RustCFML every one of those writes evaporates, so the controller
// "initializes" with no name/filters/verifications/layouts and dispatch
// fails far downstream of the real cause. Model.cfc and the csrf/caching
// mixins (variables.$class.csrf.type = ...) ride the same contract.
//
// All assertions below PASS on Lucee/ACF/BoxLang. Risky writes are wrapped
// in try/catch so the template-level RustCFML throw fails its assertions
// gracefully instead of aborting the run.
// ============================================================

// ------------------------------------------------------------
// (1) Template level, variables-scope-qualified.
// ------------------------------------------------------------
scvivTplVal = "(threw)";
scvivTplErr = "";
try {
    variables.scvivTpl.name = "viv-tpl";
    scvivTplVal = variables.scvivTpl.name;
} catch (any e) {
    scvivTplErr = e.message;
}
assert("variables.X.name on an undeclared X auto-vivifies at template level",
    scvivTplVal, "viv-tpl");
assertTrue("no exception thrown vivifying variables.X (got: [" & scvivTplErr & "])",
    len(scvivTplErr) == 0);
scvivTplIsStruct = false;
try {
    scvivTplIsStruct = StructKeyExists(variables, "scvivTpl") && IsStruct(variables.scvivTpl);
} catch (any e) {
}
assertTrue("auto-vivified variables.X is a struct", scvivTplIsStruct);

// ------------------------------------------------------------
// (2) Template level, request-scope-qualified. On RustCFML this is the
//     SILENT flavor: no throw, key registered, container not a struct,
//     member readback comes back empty.
// ------------------------------------------------------------
scvivReqVal = "(threw)";
scvivReqIsStruct = false;
try {
    request.scvivReq.name = "viv-req";
    scvivReqVal = request.scvivReq.name;
    scvivReqIsStruct = StructKeyExists(request, "scvivReq") && IsStruct(request.scvivReq);
} catch (any e) {
}
assert("request.X.name on an undeclared X auto-vivifies", scvivReqVal, "viv-req");
assertTrue("auto-vivified request.X is a struct", scvivReqIsStruct);

// ------------------------------------------------------------
// (3) Inside a function: local-scope-qualified.
// ------------------------------------------------------------
function scvivLocalFn() {
    var out = {val: "(threw)", container: false};
    try {
        local.scvivLoc.name = "viv-local";
        out.val = local.scvivLoc.name;
        out.container = IsStruct(local.scvivLoc);
    } catch (any e) {
    }
    return out;
}
scvivLocRes = scvivLocalFn();
assert("local.X.name on an undeclared X auto-vivifies inside a function",
    scvivLocRes.val, "viv-local");
assertTrue("auto-vivified local.X is a struct", scvivLocRes.container);

// ------------------------------------------------------------
// (4) variables-scope write inside a template-level function: the vivified
//     struct must land in the SHARED variables scope, visible after return.
// ------------------------------------------------------------
function scvivVarsFn() {
    try {
        variables.scvivFnv.name = "viv-fnv";
    } catch (any e) {
    }
}
scvivVarsFn();
scvivFnvVal = "(missing)";
try {
    if (StructKeyExists(variables, "scvivFnv") && IsStruct(variables.scvivFnv)) {
        scvivFnvVal = variables.scvivFnv.name;
    }
} catch (any e) {
}
assert("variables.X vivified inside a function is a struct visible after the call",
    scvivFnvVal, "viv-fnv");

// ------------------------------------------------------------
// (5) CFC method, the exact Wheels $initControllerClass shape:
//     variables.$class.name = ... with a $-prefixed key, never pre-seeded.
// ------------------------------------------------------------
scvivObj = createObject("component", "ScopedAutoVivFixture");
assert("CFC method: variables.$class.name auto-vivifies ($initControllerClass shape)",
    scvivObj.vivClassName(), "name=[vivified]");

// ------------------------------------------------------------
// (6) CFC method, two-level chain: variables.X.a.b must vivify EVERY level
//     (csrf mixin shape: variables.$class.csrf.type = ...).
// ------------------------------------------------------------
assert("CFC method: deep chain variables.X.a.b vivifies every level",
    scvivObj.vivDeep(), "b=[deep-viv]");

// ------------------------------------------------------------
// (7) Control (passes on RustCFML today): the same nested write succeeds
//     when the container IS pre-initialized -- guards the test wiring.
// ------------------------------------------------------------
variables.scvivCtl = {};
variables.scvivCtl.name = "ctl";
assert("control: nested write on a pre-initialized scoped container works",
    variables.scvivCtl.name, "ctl");

suiteEnd();
</cfscript>
