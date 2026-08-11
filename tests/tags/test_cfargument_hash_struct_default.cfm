<cfscript>
suiteBegin("Tags: cfargument unquoted hash-wrapped struct-literal default");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    A tag attribute may be given as an UNQUOTED hash-wrapped expression, and
    that expression may be a struct literal:

        <cfargument name="rf" type="struct" default=#{ "type": "json_object" }#>

    Lucee 7 evaluates the expression, so a paramless call to the function
    returns the default struct's member ("json_object"). Verified on Lucee 7.

    RustCFML fails to PARSE the attribute -- "Unterminated '#' interpolation
    in string", reported at a position away from the offending attribute --
    apparently treating the `{ "type": ... }#` tail as string-interpolation
    text instead of an expression body. The same unquoted hash-wrapped form
    with a SIMPLE expression (default=#lCase(...)#) parses fine; that is the
    control fixture.

    Parse-class gap (it would abort this whole file), so both cases live in
    runtime-instantiated fixtures: the parse failure degrades to a catchable
    createObject() throw and shows up as a clean assertion mismatch.

    Reduced from the titan (Moopa) codebase port: an LLM-call helper declares
    <cfargument name="response_format" type="struct"
    default=#{ "type": "json_object" }#>.
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

// --- control: unquoted hash-wrapped SIMPLE expression default --------------
assert("control: unquoted hash-wrapped simple-expression default parses",
    loadRun("CfargumentHashStructDefaultControlFixture"), "json_object");

// --- gap: unquoted hash-wrapped STRUCT-LITERAL default ---------------------
assert("unquoted hash-wrapped struct-literal default parses and evaluates",
    loadRun("CfargumentHashStructDefaultFixture"), "json_object");

suiteEnd();
</cfscript>
