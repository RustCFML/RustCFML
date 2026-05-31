<cfscript>
include "../harness.cfm";

suiteBegin("Server: front controller fallback");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";

if (serverPort == "" || serverPort == "0") {
    assertTrue("front controller fallback skipped (no cgi.server_port)", true);
} else {
    http
        url="http://127.0.0.1:#serverPort#/missing/path?probe=abc"
        method="GET"
        throwonerror="false"
        result="fallbackResult";

    assert("unresolved route is served by fallback template", trim(fallbackResult.fileContent), "fallback|route=/missing/path|probe=abc");
    assert("front controller fallback returns success status", fallbackResult.statusCode, "200 OK");
}

suiteEnd();
</cfscript>
