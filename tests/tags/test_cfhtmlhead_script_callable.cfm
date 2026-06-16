<cfscript>
suiteBegin("Tags: cfhtmlhead is script-callable (BIF form), not only the tag form");

// Background: cfhtmlhead exists as a TAG on RustCFML (implemented v0.186, injects into <head>
// at flush). But the cfscript STATEMENT call form cfhtmlhead(text="...") is not a callable
// builtin on RustCFML 0.190.0 — it throws "Variable 'cfhtmlhead' is undefined". Lucee/Adobe
// expose cfhtmlhead(text=) / cfhtmlhead(attributeCollection=) as a script-callable builtin
// (same as the other tag-in-script call forms). Wheels view/asset helpers call it in cfscript.

state = {called = false, err = ""};
try {
    cfhtmlhead(text = "injected-head-content");
    state.called = true;
} catch (any e) {
    state.err = e.message;
}

// the gap: cfhtmlhead(text=) must be callable in cfscript (not resolve to an undefined variable)
assertTrue("cfhtmlhead(text=) is callable in cfscript (not 'undefined')", state.called);

suiteEnd();
</cfscript>
