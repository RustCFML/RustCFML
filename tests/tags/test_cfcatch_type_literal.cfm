<cfscript>
// <cfcatch type="..."> takes any string as the type's name, as Lucee does.
//
// The name is a literal: Lucee compiles <cfcatch type="my-type">, <cfcatch type="my type"> and
// <cfcatch type="app.one,app.two">, and each catches an exception thrown with exactly that type.
// A comma makes no list: type="app.one,app.two" does not catch type "app.two".
//
// RustCFML v0.751.0 compiles the tag only when the name is a dotted identifier. Anything else
// is a parse error for the whole template ("Expected property name (found Comma)", "... (found
// Minus)", "Expected RParen, found Identifier"), so a page with such a cfcatch can't run at all.
// The script form, catch ("my-type" e), already works.
//
// Each case is its own fixture under cfcatch_type/, so one that doesn't compile fails its own
// assertions instead of this whole suite.
suiteBegin("cfcatch: the type is a literal name");

// Throw `throwType` inside the fixture; "typed" = its <cfcatch type="..."> took it, "any" = it
// fell through to the outer cfcatch type="any", else the message of what stopped the fixture.
function catchWith(required string fixture, required string throwType) {
    request.cfcatchThrowType = arguments.throwType;
    request.cfcatchWhere = "not run";
    try {
        include "cfcatch_type/" & arguments.fixture & ".cfm";
    } catch (any e) {
        return "fixture failed: " & e.message;
    }
    return request.cfcatchWhere;
}

// Control: a dotted name, which compiles today.
assert("control: type=""app.one"" catches app.one", catchWith("dotted", "app.one"), "typed");
assert("control: type=""app.one"" does not catch app.two", catchWith("dotted", "app.two"), "any");

assert("type=""my-type"" catches my-type", catchWith("hyphen", "my-type"), "typed");
assert("type=""my-type"" does not catch my", catchWith("hyphen", "my"), "any");

assert("type=""my type"" catches my type", catchWith("space", "my type"), "typed");

// A comma is part of the name, not a list separator.
assert("type=""app.one,app.two"" catches the type app.one,app.two", catchWith("comma", "app.one,app.two"), "typed");
assert("type=""app.one,app.two"" does not catch app.one", catchWith("comma", "app.one"), "any");
assert("type=""app.one,app.two"" does not catch app.two", catchWith("comma", "app.two"), "any");
assert("type=""app.one, app.two"" does not catch app.two", catchWith("comma_space", "app.two"), "any");

// The script form of the same names (works today).
function scriptCatch(required string throwType) {
    try {
        try { throw(type=arguments.throwType, message="m"); }
        catch ("my-type" e) { return "typed"; }
    } catch (any e2) { return "any"; }
}
assert("script: catch (""my-type"" e) catches my-type", scriptCatch("my-type"), "typed");
assert("script: catch (""my-type"" e) does not catch other", scriptCatch("other"), "any");

suiteEnd();
</cfscript>
