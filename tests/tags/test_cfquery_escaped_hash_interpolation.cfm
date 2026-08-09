<cfscript>
suiteBegin("Tags: escaped hash + interpolation in a cfquery body");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    Inside a cfquery body, ## is a literal hash and #expr# interpolates --
    the two compose, so a SQL string literal may contain an escaped hash
    immediately followed by an interpolation:

        <cfquery ...>SELECT 'Order ###x#' AS t</cfquery>

    yields the value "Order #5" when x=5. Verified on Lucee 7. The identical
    ###x# sequence in a cfset/cfoutput string works on BOTH engines (inline
    control below).

    RustCFML fails to PARSE the cfquery body -- "Expected RParen, found
    Identifier(\"x\")" -- so the gap is specific to how the body's text is
    lowered, not to hash escaping in general. Parse-class gap (it would abort
    this whole file), so both query cases live in runtime-instantiated
    fixtures where the parse failure degrades to a catchable createObject()
    throw. The fixtures use QoQ (dbtype="query") so they execute -- and the
    resulting VALUE is asserted, not just parseability -- with no datasource
    on either engine.

    Reduced from the titan (Moopa) codebase port: report queries build
    display strings like 'Order ###ordernum#' in SELECT lists.
    ============================================================
--->

<cfscript>
// Instantiate a fixture and run run(); returns the value when the fixture
// parsed and ran, or a diagnostic string when it did not.
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

// --- inline control: the same sequence in an ordinary string ---------------
x = 5;
assert("control: escaped hash + interpolation in a cfset string",
    "Order ###x#", "Order ##5");

// --- control fixture: plain interpolation in the query body ----------------
assert("control: plain interpolation in a cfquery-body string literal",
    loadRun("CfqueryEscapedHashControlFixture"), "Order 5");

// --- gap: escaped hash immediately before the interpolation ----------------
assert("escaped hash + interpolation in a cfquery-body string literal",
    loadRun("CfqueryEscapedHashFixture"), "Order ##5");

suiteEnd();
</cfscript>
