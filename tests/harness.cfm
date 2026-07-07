<cfscript>
// ============================================================
// RustCFML Test Harness
// ============================================================
// State stored in request scope so it persists across includes.
// Uses explicit assignment (no ++/+=) for RustCFML compatibility.
//
// Idempotent: the grand-total counters are initialised ONCE per request. This
// lets the harness be re-included safely — e.g. inside the per-test isolation
// custom tag (tests/runtest.cfm), which re-includes it to make assert()/
// suiteBegin()/suiteEnd() visible in the tag's own (isolated) variables scope
// without wiping the running totals. The first include (from runner.cfm) sets
// them up; later includes skip the reset.
if (!structKeyExists(request, "_test_totalPassed")) {
    // Grand totals
    request._test_totalPassed  = 0;
    request._test_totalFailed  = 0;
    request._test_totalSuites  = 0;
    request._test_failedSuites = 0;
    request._test_failures     = [];

    // Errored files — a test file that THROWS before reaching suiteEnd() would
    // otherwise be invisible to the grand total (its per-suite counters are only
    // folded in by suiteEnd()). suiteAbort() (called by the runtest.cfm catch)
    // records these so an aborted file can never masquerade as a green run.
    request._test_totalErrors  = 0;
    request._test_erroredFiles = [];

    // Per-suite state
    request._test_suiteName   = "";
    request._test_suitePassed = 0;
    request._test_suiteFailed = 0;
    request._test_suiteFailures = [];
    // True between suiteBegin() and suiteEnd() — lets suiteAbort() flush the
    // in-flight suite's counters when a file throws mid-suite.
    request._test_suiteOpen   = false;
}

// ---- isRustCFML() — engine detection for cross-engine (Lucee) runs ----
// The same suite runs on Lucee 7.0.4 to verify compatibility. A handful of
// assertions cover RustCFML-specific features, deliberate extensions, or
// by-design deltas (ordered structs, dateFormat single-quote literals,
// createUniqueID("counter"), cfconfig security policy + in-memory datasources).
// Those are wrapped in `if (isRustCFML())` so they exercise RustCFML but are
// skipped on Lucee, keeping a clean cross-engine bar. See
// docs/lucee-differences.md for the catalogue and the one unresolved item.
function isRustCFML() {
    return structKeyExists(server, "coldfusion")
        && structKeyExists(server.coldfusion, "productname")
        && server.coldfusion.productname == "RustCFML";
}

// ---- suiteBegin(name) ----
function suiteBegin(required string name) {
    request._test_suiteName     = arguments.name;
    request._test_suitePassed   = 0;
    request._test_suiteFailed   = 0;
    request._test_suiteFailures = [];
    request._test_suiteOpen     = true;
}

// ---- suiteEnd() ----
function suiteEnd() {
    var total = request._test_suitePassed + request._test_suiteFailed;
    request._test_totalPassed = request._test_totalPassed + request._test_suitePassed;
    request._test_totalFailed = request._test_totalFailed + request._test_suiteFailed;
    request._test_totalSuites = request._test_totalSuites + 1;

    if (request._test_suiteFailed > 0) {
        request._test_failedSuites = request._test_failedSuites + 1;
        writeOutput("FAIL | " & request._test_suiteName & " | "
            & request._test_suitePassed & "/" & total & " passed ("
            & request._test_suiteFailed & " failed)" & chr(10));
        for (var f in request._test_suiteFailures) {
            writeOutput("       FAIL: " & f & chr(10));
            arrayAppend(request._test_failures,
                request._test_suiteName & " > " & f);
        }
    } else {
        writeOutput("PASS | " & request._test_suiteName & " | "
            & request._test_suitePassed & "/" & total & " passed" & chr(10));
    }
    request._test_suiteOpen = false;
}

// ---- suiteAbort(file, message) ----
// Called by the per-test isolation tag (tests/runtest.cfm) when a test file
// throws. Without this, a mid-file exception silently drops the file's entire
// suite from the grand total — the assertions that DID run (including failures)
// vanish, the run still prints "ALL TESTS PASSED", and the process exits 0.
// That hole let a genuinely failing test ship on a tagged+pushed build.
function suiteAbort(required string file, required string message) {
    // Flush the in-flight suite first so any assertions that ran before the
    // throw (especially failures) are folded into the total and printed.
    if (request._test_suiteOpen) {
        suiteEnd();
    }
    request._test_totalErrors = request._test_totalErrors + 1;
    arrayAppend(request._test_erroredFiles, arguments.file & " | " & arguments.message);
    writeOutput("ERROR | " & arguments.file & " | " & arguments.message & chr(10));
}

// ---- assert(label, actual, expected) ----
function assert(required string label, required actual, required expected) {
    if (toString(arguments.actual) == toString(arguments.expected)) {
        request._test_suitePassed = request._test_suitePassed + 1;
    } else {
        request._test_suiteFailed = request._test_suiteFailed + 1;
        arrayAppend(request._test_suiteFailures,
            arguments.label & " | expected: [" & toString(arguments.expected)
            & "] | got: [" & toString(arguments.actual) & "]");
    }
}

// ---- assertTrue(label, value) ----
function assertTrue(required string label, required value) {
    if (arguments.value) {
        request._test_suitePassed = request._test_suitePassed + 1;
    } else {
        request._test_suiteFailed = request._test_suiteFailed + 1;
        arrayAppend(request._test_suiteFailures,
            arguments.label & " | expected truthy | got: [" & toString(arguments.value) & "]");
    }
}

// ---- assertFalse(label, value) ----
function assertFalse(required string label, required value) {
    if (!arguments.value) {
        request._test_suitePassed = request._test_suitePassed + 1;
    } else {
        request._test_suiteFailed = request._test_suiteFailed + 1;
        arrayAppend(request._test_suiteFailures,
            arguments.label & " | expected falsy | got: [" & toString(arguments.value) & "]");
    }
}

// ---- assertNull(label, value) ----
function assertNull(required string label, value) {
    if (isNull(arguments.value)) {
        request._test_suitePassed = request._test_suitePassed + 1;
    } else {
        request._test_suiteFailed = request._test_suiteFailed + 1;
        arrayAppend(request._test_suiteFailures,
            arguments.label & " | expected null | got: [" & toString(arguments.value) & "]");
    }
}

// ---- assertNotNull(label, value) ----
function assertNotNull(required string label, value) {
    if (!isNull(arguments.value)) {
        request._test_suitePassed = request._test_suitePassed + 1;
    } else {
        request._test_suiteFailed = request._test_suiteFailed + 1;
        arrayAppend(request._test_suiteFailures,
            arguments.label & " | expected not null | got null");
    }
}

// ---- assertThrows(label, callback) ----
function assertThrows(required string label, required callback) {
    try {
        callback();
        request._test_suiteFailed = request._test_suiteFailed + 1;
        arrayAppend(request._test_suiteFailures,
            arguments.label & " | expected exception | none thrown");
    } catch (any e) {
        request._test_suitePassed = request._test_suitePassed + 1;
    }
}

// ---- printSummary() ----
function printSummary() {
    var grandTotal = request._test_totalPassed + request._test_totalFailed;
    var rule = "============================================================";
    writeOutput(chr(10) & rule & chr(10));
    writeOutput("SUMMARY: " & request._test_totalPassed & "/" & grandTotal
        & " passed across " & request._test_totalSuites & " suites" & chr(10));
    if (request._test_totalFailed > 0) {
        writeOutput("FAILED:  " & request._test_totalFailed & " assertion(s) in "
            & request._test_failedSuites & " suite(s)" & chr(10));
    }
    if (request._test_totalErrors > 0) {
        writeOutput("ERRORED: " & request._test_totalErrors
            & " test file(s) aborted before completion" & chr(10));
    }

    if (request._test_totalFailed > 0 || request._test_totalErrors > 0) {
        if (request._test_totalFailed > 0) {
            writeOutput(chr(10) & "All failures:" & chr(10));
            for (var f in request._test_failures) {
                writeOutput("  - " & f & chr(10));
            }
        }
        if (request._test_totalErrors > 0) {
            writeOutput(chr(10) & "Errored files:" & chr(10));
            for (var ef in request._test_erroredFiles) {
                writeOutput("  - " & ef & chr(10));
            }
        }
        writeOutput(rule & chr(10));
        writeOutput("TEST SUITE FAILED: " & request._test_totalFailed
            & " assertion failure(s), " & request._test_totalErrors
            & " errored file(s)" & chr(10));
        writeOutput(rule & chr(10));

        // Fail the gate LOUDLY in both run modes. On the CLI an uncaught throw
        // yields a non-zero exit; in serve mode we instead set HTTP 500 (a throw
        // there would return a generic error page and lose this summary from the
        // response body). Serve is detected by the presence of a CGI server_port,
        // which the CLI does not populate.
        var isServe = structKeyExists(cgi, "server_port") && len(trim(cgi.server_port));
        if (isServe) {
            cfheader(statuscode=500, statustext="Test suite failed");
        } else {
            throw(type="TestSuiteFailed",
                message="TEST SUITE FAILED: " & request._test_totalFailed
                    & " assertion failure(s), " & request._test_totalErrors
                    & " errored file(s). See summary above.");
        }
    } else {
        writeOutput("ALL TESTS PASSED" & chr(10));
        writeOutput(rule & chr(10));
    }
}
</cfscript>
