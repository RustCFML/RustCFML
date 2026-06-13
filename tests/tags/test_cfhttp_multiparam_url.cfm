<cfscript>
suiteBegin("cfhttp: literal query string in url");

// ============================================================
// Background
// ============================================================
// A literal query string in the cfhttp url attribute --
// url="...page.cfm?a=1" or url="...page.cfm?a=1&b=2" -- must round-trip
// like any other URL. On Lucee this just works.
//
// On RustCFML a request whose url contains a literal query string fails
// SILENTLY: no exception is thrown, the result struct simply comes back with
// an empty statuscode and empty filecontent. Observed consistently when
// cfhttp runs in a request context (this suite served); intermittently from
// the CLI. The request line DOES reach the server intact (verified against
// the server debug log), so the failure is on the client's side of the
// exchange. The same parameters passed via cfhttpparam type="url" work, and
// string interpolation builds the url text correctly -- only the literal
// query-string form is broken.
//
// Behavioral round-trip: GETs a local echo target; runs only when served
// (cgi.server_port present), skips from the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("cfhttp literal query string skipped (no cgi.server_port)", true);
} else {
    target = "http://127.0.0.1:" & serverPort & "/tests/tags/cfhttp_multiparam_target.cfm";

    // control: the same parameters via cfhttpparam type="url" round-trip
    cfhttp(url="#target#", result="r1", timeout=15) {
        cfhttpparam(type="url", name="a", value="1");
        cfhttpparam(type="url", name="b", value="2");
    }
    assertTrue("control: cfhttpparam type=url params arrive", find("a=[1];b=[2]", r1.filecontent) GT 0);

    // gap: a single literal query parameter
    cfhttp(url="#target#?a=1", result="r2", timeout=15);
    assertTrue("single-param literal url returns 200", find("200", r2.statuscode ?: "") GT 0);
    assertTrue("single literal param arrives", find("a=[1]", r2.filecontent) GT 0);

    // gap: two literal query parameters
    cfhttp(url="#target#?a=1&b=2", result="r3", timeout=15);
    assertTrue("multi-param literal url returns 200", find("200", r3.statuscode ?: "") GT 0);
    assertTrue("both literal params arrive", find("a=[1];b=[2]", r3.filecontent) GT 0);
}

suiteEnd();
</cfscript>
