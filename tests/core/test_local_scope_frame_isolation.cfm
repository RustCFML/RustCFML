<cfscript>
suiteBegin("Core: local scope frame isolation");

// `local.X` is per-CALL scope — identical to `var X`. Each function invocation
// gets its OWN `local` scope, so a callee's `local.x = ...` MUST NOT alter a
// caller's `local.x` of the same name. Lucee, Adobe CF, and BoxLang all isolate
// `local` per call frame (and treat `local.x` and `var x` identically).
//
// The existing single-frame cases in compat_engine/test_scope_behavior.cfm cover
// `local.x == var x` WITHIN one frame; these cover isolation ACROSS frames.

// (1) The gap: a callee's same-named local.x leaks into the caller's frame.
function frameIsoCallee() {
    local.x = 99;
    return local.x;
}
function frameIsoCaller() {
    local.x = "ORIGINAL";
    frameIsoCallee();
    return local.x;
}
assert("callee local.x does not clobber caller local.x", frameIsoCaller(), "ORIGINAL");

// (2) Control: the `var x` form (isolated on every engine) — guards the wiring.
function frameIsoCalleeVar() {
    var x = 99;
    return x;
}
function frameIsoCallerVar() {
    var x = "ORIGINAL";
    frameIsoCalleeVar();
    return x;
}
assert("callee var x does not clobber caller var x", frameIsoCallerVar(), "ORIGINAL");

// (3) Real-world shape: a helper assigned into the caller must not corrupt the
// caller's same-named local. Mirrors framework code where a builder uses
// `local.rv` and a callee it invokes (e.g. a getter) also uses `local.rv`.
function frameIsoHelper() {
    local.rv = "HELPER";
    return local.rv;
}
function frameIsoBuild() {
    local.rv = {a: 1};
    local.other = frameIsoHelper();
    return isStruct(local.rv) ? "struct-preserved" : ("CLOBBERED:" & local.rv);
}
assert("callee local.rv does not clobber caller local.rv struct", frameIsoBuild(), "struct-preserved");

suiteEnd();
</cfscript>
