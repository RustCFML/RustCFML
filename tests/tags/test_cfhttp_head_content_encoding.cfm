<cfscript>
// cfhttp method="head" against an answer that names a Content-Encoding.
//
// A HEAD has no body, whatever Content-Encoding its answer names (the header says how the
// body a GET would get is encoded). Lucee returns the status and the headers, with an empty
// fileContent. RustCFML v0.751.0 throws "cfhttp: reading the response body failed:
// unexpected end of file": it tries to decode a body that isn't there.
//
// Real-world shape: asking an S3-compatible store whether an object exists. A HEAD for one
// that is there is answered uncompressed and works; a HEAD for a missing one is answered
// with a Content-Encoding (the error document a GET would get), so "does it exist" throws
// instead of answering no.
//
// Runs only when served (it calls a local target, cfhttp_head_encoding_target.cfm).
suiteBegin("cfhttp: HEAD with a Content-Encoding in the answer");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("cfhttp HEAD with Content-Encoding skipped (no cgi.server_port)", true);
} else {
    base = "http://127.0.0.1:" & serverPort & "/tests/tags/cfhttp_head_encoding_target.cfm";

    // Each case: one HEAD, recording what came back, or the message of what was thrown.
    cases = [
        { label: "control: HEAD without Content-Encoding", query: "", status: "200" },
        { label: "HEAD with Content-Encoding: gzip", query: "?encoding=gzip", status: "200" },
        { label: "HEAD with Content-Encoding: br", query: "?encoding=br", status: "200" },
        { label: "HEAD with Content-Encoding: deflate", query: "?encoding=deflate", status: "200" },
        // The store's case: a 404 whose answer names a Content-Encoding.
        { label: "HEAD answered 404 with Content-Encoding: gzip", query: "?encoding=gzip&status=404", status: "404" },
        // Not only HEAD: a 204 or a 304 has no body either (RFC 9110), so a GET answered
        // either way with a Content-Encoding has nothing to decode. A 304 to a conditional
        // GET from a server that compresses is the everyday shape.
        { label: "GET answered 204 with Content-Encoding: gzip", query: "?encoding=gzip&status=204", status: "204", method: "get" },
        { label: "GET answered 304 with Content-Encoding: gzip", query: "?encoding=gzip&status=304", status: "304", method: "get" }
    ];
    for (c in cases) {
        thrown = "";
        got = { status: "", content: "", reached: "" };
        try {
            cfhttp(url="#base##c.query#", method=(c.method ?: "head"), result="headResult", timeout=15);
            got.status = headResult.statuscode ?: "";
            got.content = isBinary(headResult.filecontent ?: "") ? "<binary>" : (headResult.filecontent ?: "");
            got.reached = headResult.responseheader["X-Head-Target"] ?: "";
        } catch (any e) {
            thrown = e.message;
        }
        assert(c.label & " does not throw", thrown, "");
        assert(c.label & " returns " & c.status, left(got.status, 3), c.status);
        assert(c.label & " has an empty fileContent", got.content, "");
        assert(c.label & " returns the response headers", got.reached, "reached");
    }
}

suiteEnd();
</cfscript>
