<cfscript>
suiteBegin("A component method named url() does not replace the caller's url scope");

// ============================================================
// Background
// ============================================================
// A CFC may declare a method called url() -- an S3/CDN proxy exposing
// url(key) is the common shape. On Lucee the method name is just a member of
// that component; the caller's url scope is untouched before, during and
// after createObject()/the call, and a later `url.x = ...` write is visible.
//
// On RustCFML, merely instantiating such a component empties the caller's
// url scope for the REST OF THE REQUEST: structKeyList(url) goes from
// "probe,shape" to "", url.probe becomes undefined, and `url.written = "yes"`
// silently vanishes. The same happens when the createObject() runs inside a
// function. A method named form() does not disturb the form scope, and an
// ARGUMENT named url is fine -- the trigger is specifically a method named
// after the url scope.
//
// Real-world shape (titan/Moopa): the framework instantiates every lib CFC in
// onApplicationStart; one is an S3 proxy with url(). On the request that
// re-boots the application after applicationStop(), url.route (injected by
// the front-controller fallback) is gone by the time onRequestStart runs, so
// a "?init=" re-init redirects back to the page and then dispatches "/" --
// every re-init lands on the home page.
//
// Each shape runs in its OWN request via cfhttp against probe.cfm, so the
// broken scope cannot leak into the runner's request and poison later suites.
// Runs only when served (cgi.server_port present); skips from the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

function urlMethodField(body, name) {
    var m = reFind(name & "=\[([^\]]*)\]", body, 1, true);
    if (arrayLen(m.pos) LT 2 || m.pos[2] EQ 0) return "MISSING";
    return mid(body, m.pos[2], m.len[2]);
}

function urlMethodProbe(shape) {
    var r = "";
    cfhttp(url="http://127.0.0.1:#serverPort#/tests/core/url_method_scope/probe.cfm", result="r", timeout=15) {
        cfhttpparam(type="url", name="shape", value=shape);
        cfhttpparam(type="url", name="probe", value="1");
    }
    return r.filecontent;
}

if (skip) {
    assertTrue("component method named url() skipped (no cgi.server_port)", true);
} else {
    // Shape 1: createObject() at page level, then read + write the url scope.
    b1 = urlMethodProbe("method_url");
    assert("control: url scope populated before createObject()", urlMethodField(b1, "before"), "shape,probe");
    assert("control: the method itself is callable", urlMethodField(b1, "call"), "fn-url:k");
    assert("url scope keys survive createObject() of a CFC with a url() method", urlMethodField(b1, "after_create"), "shape,probe");
    assert("url.probe still readable after createObject()", urlMethodField(b1, "probe"), "1");
    assert("a url-scope write after createObject() is visible", urlMethodField(b1, "written"), "yes");

    // Shape 2: the createObject() happens inside a function.
    b2 = urlMethodProbe("method_url_in_function");
    assert("url scope intact inside the function that created the CFC", urlMethodField(b2, "inside"), "shape,probe");
    assert("url scope intact in the caller after the function returns", urlMethodField(b2, "after"), "shape,probe");
    assert("url.probe still readable after createObject() inside a function", urlMethodField(b2, "probe"), "1");

    // Controls (green on both engines).
    b3 = urlMethodProbe("arg_url");
    assert("control: an argument named url does not disturb the url scope", urlMethodField(b3, "after_create") & "|" & urlMethodField(b3, "probe"), "shape,probe|1");
    assert("control: argument named url binds normally", urlMethodField(b3, "call"), "arg:v");

    b4 = urlMethodProbe("method_form");
    assert("control: a method named form() does not disturb the url scope", urlMethodField(b4, "after_create") & "|" & urlMethodField(b4, "probe"), "shape,probe|1");
    assert("control: method named form() is callable", urlMethodField(b4, "call"), "fn-form");
}

suiteEnd();
</cfscript>
