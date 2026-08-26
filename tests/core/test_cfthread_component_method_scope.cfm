<cfscript>
suiteBegin("cfthread inside a component method sees the component's methods");

// A <cfthread> body written inside a CFC method runs in that component's
// context on Lucee: sibling methods -- private ones included -- resolve by
// bare name and via `variables.`, the same as from the method itself.
// GH #360 (v0.630.0) stopped publishing method names into the ambient
// function table, which was right for LATER templates, but the thread body
// belongs to the component and lost them too.
//
// Real-world shape: a route method kicks off a long rebuild in a cfthread
// and calls the private worker method from the body.

ctmsFixture = createObject("component", "CfthreadMethodScopeFixture");

function ctmsRun(name) {
    try {
        return invoke(ctmsFixture, name);
    } catch (any e) {
        return "THREW:" & e.message;
    }
}

assert("bare sibling private call resolves inside the thread body",
    ctmsRun("bareSiblingCall"), "COMPLETED|10");

assert("variables.method resolves inside the thread body",
    ctmsRun("variablesScopedCall"), "COMPLETED|10");

assert("bare call whose target itself calls further private siblings",
    ctmsRun("bareSiblingCallNested"), "COMPLETED|12");

// Controls: the two shapes that already work on both engines.
assert("control: `this` passed as an attribute, public method called on it",
    ctmsRun("thisPassedAsAttribute"), "COMPLETED|10");

assert("control: UDF passed as an attribute, aliased then called bare",
    ctmsRun("functionPassedAsAttribute"), "COMPLETED|12");

suiteEnd();
</cfscript>
