<cfscript>
suiteBegin("Lifecycle: Application.cfc method overrides are case-insensitive");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";

if (serverPort == "" || serverPort == "0") {
    assertTrue("application lifecycle case override skipped (no cgi.server_port)", true);
} else {
    targetPath = "/tests/lifecycle/application_lifecycle_case_override/index.cfm";
    http url="http://127.0.0.1:#serverPort##targetPath#" method="GET" result="lifecycleResult";
    assert("application lifecycle case override status", lifecycleResult.statuscode, "200 OK");
    assert("child lifecycle override runs despite case mismatch",
        trim(lifecycleResult.filecontent),
        "parent=parent-ran|child=child-ran");
}

suiteEnd();
</cfscript>
