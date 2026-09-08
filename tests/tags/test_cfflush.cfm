<cfscript>
suiteBegin("cfflush");

// ============================================================
// In-process assertions.
//
// ONLY cases that never actually flush belong here: a real flush commits the
// response, which would freeze the runner's own headers and break every later
// cfheader/cflocation/cfcontent test in the suite. Everything that flushes for
// real is driven over HTTP against tests/tags/flush_target.cfm below.
// ============================================================

// Lucee casts `interval` to a double before the tag runs, so a non-numeric
// value is a CAST error — raised regardless of throwonerror, which only covers
// the flush itself. Verified against Lucee 7.1.
badInterval = "";
try {
    cfflush(interval="not-a-number", throwonerror=false);
} catch (any e) {
    badInterval = e.message;
}
assertTrue("non-numeric interval raises a cast error even with throwonerror=false",
    findNoCase("cast", badInterval) > 0 && findNoCase("not-a-number", badInterval) > 0);

// isFlushed() is false while nothing has been flushed. (Under the CLI it stays
// false even after a flush — there is no response to commit — which is why
// cflocation and friends keep working there. The committed case is covered over
// HTTP below.)
//
// NB the negative-interval case is deliberately NOT tested here: a negative
// interval means "flush on every write", so it would commit the runner's own
// response and leave auto-flush on for every test that follows. It runs against
// the target page instead.
assertFalse("isFlushed() is false before any flush", isFlushed());

// ============================================================
// HTTP subtests — the real flushing behaviour.
//
// Requires RustCFML running as a web server so the target is reachable; the
// port comes from cgi.server_port and the block is skipped under the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
if (serverPort == "" || serverPort == "0") {
    writeOutput(chr(10) & "  skipped HTTP subtests (no cgi.server_port — run via rustcfml --serve)" & chr(10));
} else {
    baseUrl = "http://127.0.0.1:" & serverPort;
    targetPath = "/tests/tags/flush_target.cfm";

    // --- the response commits at the first flush ---
    http url=baseUrl & targetPath & "?test=commit" method="GET" result="commitResult";
    assertTrue("flush commits the status set before it",
        left(trim(commitResult.statuscode), 3) == "201");
    assertTrue("flush commits headers set before it",
        structKeyExists(commitResult.responseheader, "X-Before")
        && commitResult.responseheader["X-Before"] == "yes");
    // Output produced after the flush must still reach the client.
    assert("post-flush output is still delivered",
        trim(commitResult.filecontent), "first-chunk|second-chunk");

    // --- flush targets the ROOT buffer, not the innermost capture ---
    // Lucee: getRootOut() == bodyContentStack.getBase(). Verified against
    // Lucee 7.1: cap comes back whole.
    http url=baseUrl & targetPath & "?test=savecontent" method="GET" result="scResult";
    assert("flush inside savecontent leaves the capture intact",
        trim(scResult.filecontent), "page-text|cap=[AB]");

    // --- interval auto-flush delivers everything, in order ---
    http url=baseUrl & targetPath & "?test=interval" method="GET" result="ivResult";
    assert("interval auto-flush delivers the whole body in order",
        trim(ivResult.filecontent),
        "segment-1;segment-2;segment-3;segment-4;segment-5;");

    // --- repeated flushes are fine (Lucee raises nothing) ---
    http url=baseUrl & targetPath & "?test=reflush" method="GET" result="rfResult";
    assert("repeated flushes are harmless", trim(rfResult.filecontent), "one|two");

    // --- what is, and is not, allowed after the response is committed ---
    // Matrix verified against Lucee 7.1 (messages match exactly).
    http url=baseUrl & targetPath & "?test=after-cookie" method="GET" result="acResult";
    assert("cfcookie after flush does not raise",
        trim(acResult.filecontent), "x|no-throw");

    http url=baseUrl & targetPath & "?test=after-header" method="GET" result="ahResult";
    assertTrue("cfheader after flush raises",
        findNoCase("already committed", ahResult.filecontent) > 0);

    http url=baseUrl & targetPath & "?test=after-content" method="GET" result="acnResult";
    assertTrue("cfcontent after flush raises",
        findNoCase("Content was already flushed", acnResult.filecontent) > 0);

    http url=baseUrl & targetPath & "?test=after-location" method="GET" redirect="false" result="alResult";
    assertTrue("cflocation after flush raises",
        findNoCase("already flushed", alResult.filecontent) > 0);
    assertTrue("cflocation after flush does not redirect",
        !structKeyExists(alResult.responseheader, "Location"));

    http url=baseUrl & targetPath & "?test=after-htmlhead" method="GET" result="ahhResult";
    assertTrue("cfhtmlhead after flush raises",
        findNoCase("already flushed", ahhResult.filecontent) > 0);

    // --- a negative/zero interval is accepted (means: flush on every write) ---
    http url=baseUrl & targetPath & "?test=neg-interval" method="GET" result="niResult";
    assert("negative and zero intervals are accepted, not errors",
        trim(niResult.filecontent), "neg-ok|zero-ok|body-intact");

    // --- isFlushed() reports the commit state (Lucee: response.isCommitted) ---
    http url=baseUrl & targetPath & "?test=isflushed" method="GET" result="ifResult";
    assert("isFlushed() is false before and true after a flush",
        trim(ifResult.filecontent), "before=false|after=true|again=true");

    // --- a flush inside cfthread must not disturb the request ---
    http url=baseUrl & targetPath & "?test=thread" method="GET" result="thResult";
    assert("flush inside cfthread leaves request output intact",
        trim(thResult.filecontent), "before|after");
}

suiteEnd();
</cfscript>
