<cfscript>
suiteBegin("Tags: arrow functions in tag-mode expressions");
</cfscript>

<!---
    ============================================================
    Background
    ============================================================
    An arrow function is an expression like any other, so it may appear in a
    TAG-MODE expression context:

        <cfset total = arr.reduce((s, r) => s + r.count, 0)>

    Lucee 7 accepts both the expression-body and the block-body
    ((s, r) => { return s + r.count; }) spellings and returns 5 for
    [{count:2},{count:3}]. Verified on Lucee 7.

    RustCFML fails to PARSE either arrow spelling in tag mode -- "Expected
    RParen, found Comma" (the parameter list is consumed as a parenthesised
    expression) -- while the IDENTICAL code inside <cfscript> works (inline
    control below) and the classic function(){} closure spelling parses fine
    in tag mode (control fixture). The gap is specific to arrow-function
    syntax in the tag-mode expression parser.

    Parse-class gap (it would abort this whole file), so both arrow shapes
    live in runtime-instantiated fixtures where the parse failure degrades to
    a catchable createObject() throw.

    Found running titan (Moopa) on v0.574.0: the tag-mode arrow spelling
    appears in legacy report code.
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

// --- inline control: the identical arrow reduce inside cfscript ------------
arr = [{count: 2}, {count: 3}];
assert("control: arrow function in cfscript",
    arr.reduce((s, r) => s + r.count, 0), 5);

// --- control fixture: classic closure spelling in tag mode -----------------
assert("control: classic function(){} closure in a tag-mode expression",
    loadRun("TagModeArrowControlFixture"), 5);

// --- gap: arrow function (expression body) in tag mode ---------------------
assert("arrow function (expression body) parses in a tag-mode expression",
    loadRun("TagModeArrowFixture"), 5);

// --- gap: arrow function (block body) in tag mode --------------------------
assert("arrow function (block body) parses in a tag-mode expression",
    loadRun("TagModeArrowBlockFixture"), 5);

suiteEnd();
</cfscript>
