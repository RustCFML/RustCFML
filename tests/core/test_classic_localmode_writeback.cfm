<cfscript>
suiteBegin("Classic localMode parent-scope writeback");

/*
 * v0.599 — the frame epilogue skips the classic-localMode parent-scope
 * writeback diff when the frame never mutated its locals map (the diff cannot
 * find anything in that case). On live Preside that diff ran on 98% of frames
 * and scanned 3.36M keys to produce 38 writes.
 *
 * The failure mode of getting that guard wrong is SILENT — writes simply stop
 * propagating and nothing throws — so these assertions pin the semantics the
 * guard must preserve. Classic localMode is the DEFAULT (`this.localMode`
 * unset), which is what this file runs under.
 */

function wbInner() {
    wbCounter = wbCounter + 1;      // undeclared bareword: propagates out
    wbFresh   = "made-in-inner";    // brand-new undeclared key: also propagates
}
function wbVarOnly() {
    var wbScoped = "stays";         // var-declared: must NOT propagate
}
function wbNoWrites() {
    return wbCounter;               // mutates nothing: guard skips its diff
}
function wbNested() {
    wbInner();                      // propagation must survive one frame deeper
}

wbCounter = 1;
wbInner();
assert("undeclared write propagates to caller", wbCounter, 2);
assert("brand-new undeclared key propagates", isDefined("wbFresh") ? wbFresh : "MISSING", "made-in-inner");

wbVarOnly();
assertFalse("var-declared local does NOT leak out", isDefined("wbScoped"));

assert("a frame that writes nothing still reads through", wbNoWrites(), 2);

// A no-write frame between two writing frames must not break the chain: this is
// the case the guard actually changes (its diff is skipped entirely).
wbInner();
assert("propagation still works after a skipped frame", wbCounter, 3);

wbNested();
assert("propagation works through a nested call", wbCounter, 4);

suiteEnd();
</cfscript>
