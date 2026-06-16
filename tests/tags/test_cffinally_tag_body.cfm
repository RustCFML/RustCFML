<cfscript>
suiteBegin("Tags: cffinally tag body");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    The tag form <cftry> ... <cfcatch> ... <cffinally> ... </cftry> and the
    catchless <cftry> ... <cffinally> ... </cftry> form both transpile to
    CFScript try/finally shapes. On Lucee 5/6/7, Adobe ColdFusion, and BoxLang
    the finally block parses and always runs (after normal completion and after
    a caught exception).

    On RustCFML the whitespace (newlines/indentation) between </cfcatch> and
    <cffinally> -- and between a <cftry> body and <cffinally> -- was emitted as
    __writeText(...) into the structural gap of the generated try statement,
    producing "} __writeText(); finally {" which fails to parse. Only a
    single-line form with no inter-tag whitespace happened to work; every
    normally-formatted try/catch/finally did not compile.

    Because that is a hard PARSE error (and a parse error escapes try/catch and
    would abort the whole runner), the cases live in runtime-instantiated
    fixture CFCs -- the same pattern as test_tag_attribute_interpolation.cfm /
    test_component_declaration_attributes.cfm. An unparseable fixture makes
    createObject() throw "Could not find the component"; loadRun() catches that
    and returns the message, so the gap shows as a clean failed assertion rather
    than taking down the run. The fixtures are deliberately multi-line: a
    single-line fixture would parse even on the unfixed engine and hide the gap.

    Why it matters for Moopa: code/moopa/lib/cloudflare_stream.cfc (uploadCaptionVtt)
    uses a <cffinally> block to delete its temp upload file; the parse failure
    made /moopa/lib/cloudflare_stream "not found".
    ============================================================
--->

<cfscript>
// Instantiate a fixture and run run(); returns the step trace when the fixture
// parsed and ran, or a diagnostic string when it did not (so a parse failure
// becomes a clean assertion mismatch rather than an aborted suite).
function loadRun(required string name) {
    try {
        var o = createObject("component", arguments.name);
        if (!isObject(o)) {
            return "NOT-A-COMPONENT";
        }
        return o.run();
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

// --- control: try/catch with no finally already parses (regression guard) ----

assert("control: multi-line try/catch (no finally) parses and runs",
    loadRun("CffinallyControlFixture"), "try");

// --- gaps: try/catch/finally with normal multi-line formatting ---------------

assert("try/catch/finally (no exception) parses and runs finally",
    loadRun("CffinallyHappyPathFixture"), "try,finally");
assert("try/catch/finally (caught exception) runs catch then finally",
    loadRun("CffinallyAfterThrowFixture"), "try,catch,finally");
assert("try/finally without catch parses and runs finally",
    loadRun("CffinallyWithoutCatchFixture"), "try,finally");

suiteEnd();
</cfscript>
