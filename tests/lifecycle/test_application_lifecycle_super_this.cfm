<cfscript>
suiteBegin("Lifecycle: Application.cfc super calls bind this");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";

if (serverPort == "" || serverPort == "0") {
    assertTrue("application lifecycle super this skipped (no cgi.server_port)", true);
} else {
    targetPath = "/tests/lifecycle/application_lifecycle_super_this/index.cfm";
    http url="http://127.0.0.1:#serverPort##targetPath#" method="GET" result="lifecycleResult";
    assert("application lifecycle super this status", lifecycleResult.statuscode, "200 OK");
    assert("inherited lifecycle super call keeps this scope",
        trim(lifecycleResult.filecontent),
        "parent=parent-this|child=child-ran");
}

suiteEnd();
</cfscript>
