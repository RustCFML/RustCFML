<cfscript>
// GH #282: an undefined-variable read that throws from INSIDE a component
// method/UDF must carry type "expression" (the CFML convention Lucee/ACF use),
// same as a read at page scope. RustCFML previously downgraded the cross-frame
// error to "Runtime" because the callee's try-stack is empty (the page-level
// handler lives in an outer frame), so the fall-through raised a `runtime`
// error instead of preserving `expression`. This broke `catch (expression e)`
// / `e.type == "expression"` handlers written to the CFML convention — it
// surfaced in FW/1's InjectPropertiesTest::testInjectWithType.
suiteBegin("Undefined read inside method throws type 'expression' (GH ##282)");

bean = new oop.UndefinedReadProbe();

// --- explicitly-scoped undefined read inside a method (cross-frame catch) ---
scopedType = "";
try { bean.readVarScoped(); } catch (any e) { scopedType = e.type; }
assert("variables.x undefined inside method -> expression", scopedType, "expression");

// --- bare undefined read inside a method (cross-frame catch) ---
bareType = "";
try { bean.readUnscoped(); } catch (any e) { bareType = e.type; }
assert("bare x undefined inside method -> expression", bareType, "expression");

// --- page-scope undefined read (was already correct; guards against regressing) ---
pageType = "";
try { y = pageLevelMissing; } catch (any e) { pageType = e.type; }
assert("undefined read at page scope -> expression", pageType, "expression");

// --- same-frame (in-handler) catch inside the method reports the same type ---
assert("in-method same-frame catch reports expression", bean.readAndReportType(), "expression");

// --- typed `catch (expression e)` clause matches the cross-frame error ---
caughtTyped = false;
try {
    bean.readUnscoped();
} catch (expression e) {
    caughtTyped = true;
} catch (any e) {
    caughtTyped = false;
}
assertTrue("catch (expression e) matches undefined read from inside method", caughtTyped);

suiteEnd();
</cfscript>
