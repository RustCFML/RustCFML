<cfscript>
suiteBegin("Lifecycle: super.method() from an Application.cfc pseudo-constructor sees the method table");

// Regression (v0.630.0, found booting Preside): an Application.cfc whose entire
// body is `super.setupApplication( ... )` never materializes a `this`, so the
// super-dispatch fell back to the `__is_super` struct when deciding what to bind
// as the parent method's `__variables`. That struct carries methods but has no
// `__variables` key, so the parent method ran with NO variables scope at all.
//
// It went unnoticed until v0.630.0 because a bare sibling call used to resolve
// against the GLOBAL user-function table that every component published into.
// GH #360 stopped that leak (correctly — a CFC's private method names must not
// be ambient), leaving this frame with nothing to resolve against. Preside's
// Bootstrap.cfc died evaluating its own default argument:
//   `array statelessUrlPatterns = _getDefaultStatelessUrlPatterns()`
//   -> "Variable '_getDefaultStatelessUrlPatterns' is undefined"
//
// The fixture asserts all three ways the method table is reached from that frame:
// a bare call in a default argument, an explicit `variables.`-qualified call in a
// default argument, and a bare call from the method body.

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";

if (serverPort == "" || serverPort == "0") {
    assertTrue("super-pseudoctor variables scope skipped (no cgi.server_port)", true);
} else {
    targetPath = "/tests/lifecycle/application_super_pseudoctor_vars/sub/page.cfm";
    http url="http://127.0.0.1:#serverPort##targetPath#" method="GET" result="spResult";

    assert("super-pseudoctor request status", spResult.statuscode, "200 OK");
    assert("bare sibling call in a default argument resolves, and this.name from the super call took effect",
        trim(spResult.filecontent),
        "patterns=^/api/,^/asset/;viaVariables=^/api/;fromBody=true;name=rustcfml-super-pseudoctor-vars-test");
}

suiteEnd();
</cfscript>
