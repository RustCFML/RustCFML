<cfscript>
suiteBegin("Core: bare call to a UDF named 'new' dispatches; method call on null throws");

// Two RUNTIME silent-failure gaps that compose into the worst failure mode —
// a framework API that reports success without doing anything:
//
// (C1) A bare sibling call to a UDF literally named `new` must dispatch to
//      that UDF. `new` is a soft keyword: with no component path after it,
//      `new (argumentCollection = {...})` is a plain function call, not the
//      new-operator.
//
//          component {
//              public any function new(struct properties = {}) { return {...}; }
//              public any function createViaBare() {
//                  return new (argumentCollection = {properties = {a = 1}});
//              }
//          }
//
//          o.createViaBare()
//            RustCFML 0.105.0 -> NULL (the UDF never runs)
//            Lucee 5.4.8.2    -> the struct returned by the UDF
//          o.createViaThis() — this.new(argumentCollection=...) — works on BOTH.
//
// (C2) A method call on a null receiver must throw.
//
//          r = <null>; r.save(a = 1)
//            RustCFML 0.105.0 -> silently evaluates to null (no error)
//            Lucee 5.4.8.2    -> throws
//
// Why it matters for Wheels: model creation is exactly this composition
// (vendor/wheels/model/create.cfc:32):
//
//          local.rv = new (argumentCollection = arguments);
//          local.rv.save(...);
//          return local.rv;
//
// On RustCFML the `new()` UDF never ran (C1: local.rv = null), `.save()` on
// the null no-op'd (C2: no throw), and create() returned "success" while no
// row was ever written. Either gap alone would at least be loud; composed,
// they produce silent fake success.

gapCFixture = new NewUdfFixture();

// --- (C1) bare new(argumentCollection=...) dispatches to the sibling UDF ---
gapCBareResult = gapCFixture.createViaBare();
assertTrue("bare new() returned a value (the UDF ran)", !isNull(gapCBareResult));
if (!isNull(gapCBareResult) && isStruct(gapCBareResult) && structKeyExists(gapCBareResult, "marker")) {
    assert("bare new() dispatched to the sibling UDF (marker)",
        gapCBareResult.marker, "NEW-RAN");
    assert("argumentCollection was forwarded through the bare call",
        gapCBareResult.props.a, 1);
} else {
    assert("bare new() dispatched to the sibling UDF (marker)",
        "(no struct returned)", "NEW-RAN");
    assert("argumentCollection was forwarded through the bare call",
        "(no struct returned)", 1);
}

// --- (C1 CONTROL) explicit this.new(argumentCollection=...) — green on both ---
gapCThisResult = gapCFixture.createViaThis();
if (!isNull(gapCThisResult) && isStruct(gapCThisResult) && structKeyExists(gapCThisResult, "marker")) {
    assert("CONTROL: this.new() dispatched to the UDF (marker)",
        gapCThisResult.marker, "NEW-RAN");
} else {
    assert("CONTROL: this.new() dispatched to the UDF (marker)",
        "(no struct returned)", "NEW-RAN");
}

// --- (C2) a method call on a null receiver must throw ---
// Fresh, uniquely-named state struct: flag persistence across catch blocks
// has engine-specific quirks, and the runner shares one variables scope
// across every included test.
gapCNullCallState = { threw = false, observed = "unset" };
gapCNullReceiver = gapCFixture.giveNothing(); // returns nothing -> null receiver
try {
    gapCNullOutcome = gapCNullReceiver.save(a = 1);
    gapCNullCallState.observed = isNull(gapCNullOutcome) ? "(null)" : toString(gapCNullOutcome);
} catch (any e) {
    gapCNullCallState.threw = true;
}
assertTrue("method call on a null receiver throws", gapCNullCallState.threw);

// --- (C2 CONTROL) the same method call on a real receiver — green on both ---
assert("CONTROL: save(a=1) on a real object works", gapCFixture.save(a = 1), "SAVED:1");

suiteEnd();
</cfscript>
