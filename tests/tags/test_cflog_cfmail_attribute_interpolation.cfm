<cfscript>
suiteBegin("Tags: cflog / cfmail tag-attribute interpolation");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    A quoted tag-attribute value must evaluate #...# interpolation while
    preserving the literal text before, between, and after each interpolated
    segment, on Lucee 5/6/7, Adobe ColdFusion, and BoxLang. The sibling suite
    test_tag_attribute_interpolation.cfm pins this for cfthrow / cfargument /
    cffile / cfparam, and its header asserts the rule holds "regardless of which
    tag the attribute belongs to."

    Two tags slipped through that cross-tag claim: the <cflog> and <cfmail> TAG
    forms. Their attribute values are still routed through strip_hashes(), which
    only handles a value that is ENTIRELY a single #expr#. A value that mixes
    literal text with interpolation (e.g. text="hello #who# end") emits malformed
    script, so the containing template fails to PARSE.

    This is NOT the same failure mode as the cfthrow cases: cfthrow's old path
    still compiled (it just emitted the wrong, un-interpolated string), so those
    gaps could be pinned inline with <cftry> and a value assertion. A
    cflog/cfmail mixed-interpolation value is a hard PARSE error, and a parse
    error escapes try/catch and would abort the whole runner. So -- as with
    test_component_declaration_attributes.cfm and test_mapping_include.cfm -- the
    offending tags live in runtime-instantiated FIXTURE CFCs. An unparseable
    fixture makes createObject() throw "Could not find the component"; loadEmit()
    catches that and returns the message, so the gap shows as a clean failed
    assertion instead of taking down the run.

    Why it matters for Moopa: code/moopa/internal/routing/access_policy.cfc logs
      <cflog type="information" file="security_check"
             text="Security check for route #...# and endpoint #...# took #getTickCount() - start_time#ms">
    On RustCFML that file fails to parse, so /moopa/internal/routing/access_policy
    is "not found", moo_route cannot construct, and every secured route 500s.

    Scope: these assertions verify PARSE + execution (the failure mode is a
    compile error). They intentionally do not read back the logged text or sent
    message -- log/mail destinations differ across engines, and the existing
    merged cflog test (test_tags_cfscript_statements.cfm) sets the same bar.
    ============================================================
--->

<cfscript>
// Instantiate a fixture and run emit(); returns the sentinel "ok" when the
// fixture parsed and ran, or a diagnostic string when it did not (so a parse
// failure becomes a clean assertion mismatch rather than an aborted suite).
function loadEmit(required string name) {
    try {
        var o = createObject("component", arguments.name);
        if (!isObject(o)) {
            return "NOT-A-COMPONENT";
        }
        return o.emit();
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

// --- controls: shapes RustCFML already parses (regression guards) ------------

assert("control: cflog single bare interpolation attribute parses and runs",
    loadEmit("CflogTagAttrInterpControlFixture"), "ok");
assert("control: cfmail single bare interpolation attributes parse",
    loadEmit("CfmailTagAttrInterpControlFixture"), "ok");

// --- gaps: literal text mixed with interpolation in a tag attribute ----------

assert("cflog text: literal + interpolation + function-call segment parses",
    loadEmit("CflogTagAttrInterpFixture"), "ok");
assert("cfmail subject: literal + interpolation + function-call segment parses",
    loadEmit("CfmailTagAttrInterpFixture"), "ok");

suiteEnd();
</cfscript>
