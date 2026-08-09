<cfscript>
suiteBegin("Tags: cfparam name with a bracket key containing a colon");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    cfparam's name= is an lvalue PATH, and a bracket segment holds a quoted
    string key -- any string, including one with a colon:

        <cfset item = {}>
        <cfparam name="item['x-bind:href']" default="x">

    (The key shape is an Alpine.js bound-attribute name, e.g. x-bind:href.)
    Lucee parses the path and creates the key; item['x-bind:href'] reads "x".
    Verified on Lucee 7.

    RustCFML fails to PARSE the name path -- "Expected RBracket, found
    Colon" -- even though the SAME key works in ordinary expressions
    (item['x-bind:href'] = "x" assigns and reads fine; pinned as an inline
    control below). Parse-class gap (it would abort this whole file), so the
    failing shape lives in a runtime-instantiated fixture where the parse
    failure degrades to a catchable createObject() throw. The control fixture
    uses a dotted name path (name="item.href"), which RustCFML already
    handles.

    Reduced from the titan (Moopa) codebase port: form templates cfparam
    Alpine x-bind:* attribute slots in an attribute struct.
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

// --- inline control: the same key in an ordinary expression ----------------
item2 = {};
item2["x-bind:href"] = "x";
assert("control: colon key works in ordinary bracket expressions",
    item2["x-bind:href"], "x");

// --- control fixture: dotted cfparam name path -----------------------------
assert("control: dotted cfparam name path parses and applies",
    loadRun("CfparamBracketColonControlFixture"), "y");

// --- gap: bracket key containing a colon -----------------------------------
assert("cfparam name with a colon-bearing bracket key parses and applies",
    loadRun("CfparamBracketColonFixture"), "x");

suiteEnd();
</cfscript>
