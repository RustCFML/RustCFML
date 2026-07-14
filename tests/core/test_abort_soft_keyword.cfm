<cfscript>
suiteBegin("Core: `abort` is a soft keyword");

// ============================================================
// Background
// ============================================================
// `abort` is a SOFT keyword on Lucee/Adobe ColdFusion/BoxLang: it is the abort
// statement ONLY when used as a bare statement (`abort;`, `abort "msg";`) or via
// the <cfabort> tag. Used as an ordinary identifier it is legal — as a variable
// name, an assignment target, a struct key, and a member name.
//
// RustCFML used to treat `abort` as a HARD keyword: the parser committed to the
// abort statement the moment it saw the token, so `abort = "x"` (bare
// assignment) and `<cfset abort = "x">` were silently swallowed as an abort —
// discarding the RHS and terminating the request with a blank page and NO error.
// This is exactly how Masa CMS's setup `cleanIni()` (which uses a local
// `var abort = "<cfabort/>"`) silently killed the fresh-DB installer.
// These tests pin the soft-keyword behavior and guard that genuine aborts still
// abort.
// ============================================================

// --- `abort` as an ordinary variable -------------------------------------

abort = "widget";
assert("bare `abort = x` assignment then read", abort, "widget");

abort = abort & "-2";
assert("bare `abort` compound-ish reassignment", abort, "widget-2");

abort &= "!";
assert("`abort &=` concat-assign", abort, "widget-2!");

// --- `abort` as a local var inside a function (the Masa cleanIni case) ----

function makeAbort() {
    var abort = "";
    abort = "<cf" & "abort/>";
    return abort;
}
assert("`var abort` local then assign then return", makeAbort(), "<cfabort/>");

// --- `abort` as a struct key / member ------------------------------------

s = {};
s.abort = "hi";
assert("struct member .abort write/read", s.abort, "hi");
assert("struct bracket-key abort read", s["abort"], "hi");

// --- tag-form <cfset abort = ...> ----------------------------------------
</cfscript>
<cfset abort = "tagval">
<cfscript>
assert("tag `<cfset abort = x>` assignment", abort, "tagval");

// NOTE: the genuine `abort;` statement / <cfabort> tag are deliberately NOT
// exercised here — abort is a hard, non-catchable request termination, so
// running it would kill the whole test runner. Their behavior is covered by the
// engine's ubiquitous existing `abort;` usage and the fix's own guard (a bare
// `abort` not followed by `= . [ (` still takes the statement path).

suiteEnd();
</cfscript>
