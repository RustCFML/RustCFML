<cfscript>
// Threshold-gated sampling profiler (Phase 2 of the observability plan).
//
// The profiler samples a request's CFML call stack when a watchdog thread
// (serve mode only) decides it has run past the threshold. That timing path
// can't be exercised deterministically from the test runner, so this suite
// drives the synchronous surface instead: `profileNow()` forces one immediate
// sample of the current stack, and `getRequestProfile()` reports the folded
// call tree. tests/.cfconfig.json arms the profiler (observability.profiler
// .enabled), and the CLI run path installs a hub with no watchdog — so
// profileNow() is the only thing that captures, keeping this fully
// deterministic. RustCFML-guarded (Lucee has no such BIFs).
suiteBegin("Sampling profiler BIFs");

if (isRustCFML()) {
    // Before any sample, the profile is empty.
    var empty = getRequestProfile();
    assert("getRequestProfile returns a struct", isStruct(empty), true);

    // Force a sample from inside a known call chain so the captured stack
    // contains our function frames (not just top-level page code).
    function inner() {
        return profileNow();
    }
    function outer() {
        return inner();
    }

    var armed = outer();
    assert("profileNow returns boolean", isBoolean(armed), true);

    // The profiler is a server-level subsystem: it is armed only when the
    // running instance's startup config enables it (the hub + watchdog are
    // process-global, not per-folder). The test runner's .cfconfig.json enables
    // it, so under the CLI runner `armed` is true and we assert the full call
    // tree. In a serve instance whose startup config leaves it off, profileNow()
    // returns false and getRequestProfile() is empty — assert that graceful path
    // instead. Either way the BIFs never error.
    if (armed) {
        var prof = getRequestProfile();
        assert("profile has a sampleCount", structKeyExists(prof, "sampleCount"), true);
        assert("at least one sample captured", prof.sampleCount >= 1, true);
        assert("profile has a root node", structKeyExists(prof, "root"), true);

        // The root's total equals the sample count; children are the callees.
        assert("root total matches sampleCount", prof.root.total == prof.sampleCount, true);
        assert("root has children array", isArray(prof.root.children), true);

        // Walk the tree collecting function names; our call chain must appear.
        names = [];
        function collect(node) {
            arrayAppend(names, node.function);
            for (child in node.children) {
                collect(child);
            }
        }
        collect(prof.root);
        assert("captured stack includes outer()", arrayFindNoCase(names, "outer") > 0, true);
        assert("captured stack includes inner()", arrayFindNoCase(names, "inner") > 0, true);

        // A second forced sample increments the count.
        var n1 = getRequestProfile().sampleCount;
        profileNow();
        assert("second profileNow adds a sample", getRequestProfile().sampleCount == n1 + 1, true);
    } else {
        // Profiler off server-wide: getRequestProfile() is an empty struct.
        assert("getRequestProfile empty when profiler off",
            structKeyExists(getRequestProfile(), "sampleCount"), false);
    }
}

suiteEnd();
</cfscript>
