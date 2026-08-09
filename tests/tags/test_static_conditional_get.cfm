<cfscript>
// ============================================================
// Static asset conditional GET (Last-Modified / ETag / 304)
//
// The serve-mode static handler emits mtime-derived validators and answers
// If-None-Match / If-Modified-Since revalidations with 304 instead of
// re-reading and re-sending the body. Assertions are validator-conditional
// so the suite also passes on engines/servers with their own static
// handling (Lucee under CommandBox serves statics via undertow).
// Discover the live port from cgi.server_port; skip under the CLI runner.
// ============================================================
suiteBegin("Static conditional GET");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
if (serverPort == "" || serverPort == "0") {
    writeOutput(chr(10) & "  skipped HTTP subtests (no cgi.server_port — run via rustcfml --serve)" & chr(10));
    assert("static conditional GET (skipped, no server)", true, true);
} else {
    assetUrl = "http://127.0.0.1:" & serverPort & "/tests/tags/literal_styles.css";

    http url=assetUrl method="GET" result="firstGet";
    assertTrue("static asset serves 200", left(firstGet.statuscode, 3) == "200");

    etagVal = structKeyExists(firstGet.responseheader, "ETag") ? firstGet.responseheader["ETag"] : "";
    lastMod = structKeyExists(firstGet.responseheader, "Last-Modified") ? firstGet.responseheader["Last-Modified"] : "";

    if (len(etagVal)) {
        http url=assetUrl method="GET" result="inmGet" {
            httpparam type="header" name="If-None-Match" value="#etagVal#";
        }
        assertTrue("If-None-Match revalidation answers 304", left(inmGet.statuscode, 3) == "304");
        assertTrue("304 body is empty", len(trim(inmGet.filecontent)) == 0);
    } else {
        assert("If-None-Match (skipped, no ETag emitted)", true, true);
    }

    if (len(lastMod)) {
        http url=assetUrl method="GET" result="imsGet" {
            httpparam type="header" name="If-Modified-Since" value="#lastMod#";
        }
        assertTrue("If-Modified-Since revalidation answers 304", left(imsGet.statuscode, 3) == "304");
    } else {
        assert("If-Modified-Since (skipped, no Last-Modified emitted)", true, true);
    }

    // A stale validator must still get the full body.
    http url=assetUrl method="GET" result="staleGet" {
        httpparam type="header" name="If-Modified-Since" value="Mon, 01 Jan 2001 00:00:00 GMT";
    }
    assertTrue("stale If-Modified-Since serves 200", left(staleGet.statuscode, 3) == "200");
    assertTrue("stale revalidation carries the body", len(staleGet.filecontent) > 0);
}

suiteEnd();
</cfscript>
