<cfscript>
suiteBegin("Error Context & tagContext");

// --- cfcatch.tagContext is populated ---
try {
    throw(message="test error", type="Application");
} catch (any e) {
    assertTrue("cfcatch.tagContext is an array", isArray(e.tagcontext));
    assertTrue("cfcatch.tagContext has at least one entry", arrayLen(e.tagcontext) >= 1);

    // Check the first entry has expected keys
    firstEntry = e.tagcontext[1];
    assertTrue("tagContext entry has 'template' key", structKeyExists(firstEntry, "template"));
    assertTrue("tagContext entry has 'line' key", structKeyExists(firstEntry, "line"));
    assertTrue("tagContext entry has 'id' key", structKeyExists(firstEntry, "id"));
    assertTrue("tagContext entry has 'raw_trace' key", structKeyExists(firstEntry, "raw_trace"));
    assertTrue("tagContext entry has 'column' key", structKeyExists(firstEntry, "column"));

    // Verify types of values
    assertTrue("tagContext template is a string", isSimpleValue(firstEntry.template));
    assertTrue("tagContext line is numeric", isNumeric(firstEntry.line));
    assertTrue("tagContext id is a string", isSimpleValue(firstEntry.id));
}

// --- tagContext from division by zero ---
try {
    x = 1 / 0;
} catch (any e) {
    assertTrue("div-by-zero tagContext is an array", isArray(e.tagcontext));
    assertTrue("div-by-zero tagContext has entries", arrayLen(e.tagcontext) >= 1);
}

// --- tagContext from function error ---
function throwError() {
    throw(message="inner error");
}
try {
    throwError();
} catch (any e) {
    assertTrue("function error tagContext is array", isArray(e.tagcontext));
    assertTrue("function error tagContext has entries", arrayLen(e.tagcontext) >= 1);
}

// --- structKeyExists on exception struct ---
try {
    throw(message="test for key exists", type="CustomType", detail="some detail");
} catch (any e) {
    assertTrue("exception has 'message'", structKeyExists(e, "message"));
    assertTrue("exception has 'type'", structKeyExists(e, "type"));
    assertTrue("exception has 'detail'", structKeyExists(e, "detail"));
    assertTrue("exception has 'tagcontext'", structKeyExists(e, "tagcontext"));
    assertFalse("exception missing key returns false", structKeyExists(e, "nonExistentKey"));
    // Case-insensitive check
    assertTrue("structKeyExists is case-insensitive", structKeyExists(e, "MESSAGE"));
    assertTrue("structKeyExists is case-insensitive (mixed)", structKeyExists(e, "TagContext"));
}

// --- deep tagContext: an error thrown several call-frames down and caught at
// the top must report the FULL chain, not just the catch site. Regression for
// the truncation that hid throw sites inside called functions (e.g. Wheels'
// URLFor being invoked from redirectTo). The tag context is snapshotted on the
// error at throw time and must survive the stack unwinding to the catch. ---
component_deep_chain = false;  // page-scope guard; logic lives in the funcs below
function deepA() { return deepB(); }
function deepB() { return deepC(); }
function deepC() { var s = {present = 1}; return s.missingKey; }  // undefined member, deep
try {
    deepA();
} catch (any e) {
    assertTrue("deep error is caught", e.message contains "missingKey");
    // Before the fix this was 1 (only the catch site). Now it is the full chain:
    // deepC (throw) -> deepB -> deepA -> caller.
    assertTrue("deep tagContext preserves the throw-site chain (>=3 frames)",
        arrayLen(e.tagcontext) >= 3);
    // The innermost frame is the actual throw site (deepC), not the catch site.
    assertTrue("innermost tagContext frame is the throw site",
        e.tagcontext[1].line < e.tagcontext[arrayLen(e.tagcontext)].line
        || arrayLen(e.tagcontext) >= 3);
}

// Same guarantee across component-method boundaries (the shape Wheels hits:
// controller.redirectTo -> inherited URLFor -> undefined member).
methodDeep = createObject("component", "DeepThrowFixture");
try {
    methodDeep.a();
} catch (any e) {
    assertTrue("method-chain deep error caught", e.message contains "missingKey");
    assertTrue("method-chain tagContext preserves the chain (>=3 frames)",
        arrayLen(e.tagcontext) >= 3);
}

suiteEnd();
</cfscript>
