<cfscript>
suiteBegin("Sessions are namespaced by application name");

// ============================================================
// Background
// ============================================================
// In CFML, session identity is the composite (application name, session id).
// Application.cfc's pseudo-constructor runs per request and `this.name`
// selects the application -- one server instance can host several apps, each
// with an isolated application AND session scope. Lucee resolves the same
// CFID presented under two different `this.name` values to two independent
// sessions, and onSessionStart fires per (application, session).
//
// RustCFML namespaces the application scope correctly (ApplicationStore is
// keyed by app name) but sessions are keyed by the bare session id, so two
// applications in one process that receive the same CFID share -- and
// corrupt -- one session. Browsers do not scope cookies by port, so
// localhost:8180 (app A) and localhost:8181 (app B) send the identical CFID;
// any copied CFID is also valid across applications (session-fixation-shaped).
//
// Behavioral round-trip: two fixture apps with distinct `this.name` live under
// session_namespace/a and /b. Every probe WRITES to the session so the test is
// agnostic to lazy session creation (post-v0.131.0 default: record, cookie and
// onSessionStart happen on first session write). Runs only when served
// (cgi.server_port present); skips from the CLI. Query params are passed via
// cfhttpparam type="url" -- see the companion multi-param cfhttp test.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

function sessNsCookieHeader(resp) {
    var sc = resp.responseheader["Set-Cookie"] ?: "";
    var raw = isArray(sc) ? sc : [sc];
    var pairs = [];
    for (var c in raw) {
        var firstPart = trim(listFirst(c, ";"));
        if (len(firstPart)) arrayAppend(pairs, firstPart);
    }
    return arrayToList(pairs, "; ");
}

if (skip) {
    assertTrue("session app-namespace skipped (no cgi.server_port)", true);
} else {
    base = "http://127.0.0.1:" & serverPort & "/tests/lifecycle/session_namespace";

    // r1: write to app A with no cookie; capture the session cookie it mints.
    cfhttp(url="#base#/a/page.cfm", result="r1", timeout=15) {
        cfhttpparam(type="url", name="op", value="write");
        cfhttpparam(type="url", name="val", value="alpha");
    }
    cookieHeader = sessNsCookieHeader(r1);
    assertTrue("app A mints a session cookie on first write", len(cookieHeader) GT 0);
    assertTrue("control: app A onSessionStart ran in A", find("started=[A]", r1.filecontent) GT 0);

    // r2 (control): same cookie back to app A -- session persists in-app.
    cfhttp(url="#base#/a/page.cfm", result="r2", timeout=15) {
        cfhttpparam(type="header", name="Cookie", value=cookieHeader);
        cfhttpparam(type="url", name="op", value="read");
    }
    assertTrue("control: app A sees its own session value", find("x=[alpha]", r2.filecontent) GT 0);

    // r3 (gap): the SAME cookie written to in app B must be a DIFFERENT
    // session: onSessionStart fires for (app B, this id), not inherited
    // from app A's session.
    cfhttp(url="#base#/b/page.cfm", result="r3", timeout=15) {
        cfhttpparam(type="header", name="Cookie", value=cookieHeader);
        cfhttpparam(type="url", name="op", value="write");
        cfhttpparam(type="url", name="val", value="beta");
    }
    assertTrue("onSessionStart fires per (application, session)", find("started=[B]", r3.filecontent) GT 0);

    // r4 (gap): app B's write must not leak into app A's session.
    cfhttp(url="#base#/a/page.cfm", result="r4", timeout=15) {
        cfhttpparam(type="header", name="Cookie", value=cookieHeader);
        cfhttpparam(type="url", name="op", value="read");
    }
    assertTrue("app B write does not leak into app A", find("x=[alpha]", r4.filecontent) GT 0);

    // r5 (control): app B's own session persists independently.
    cfhttp(url="#base#/b/page.cfm", result="r5", timeout=15) {
        cfhttpparam(type="header", name="Cookie", value=cookieHeader);
        cfhttpparam(type="url", name="op", value="read");
    }
    assertTrue("app B keeps its own session value", find("x=[beta]", r5.filecontent) GT 0);
}

suiteEnd();
</cfscript>
