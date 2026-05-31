<cfscript>
suiteBegin("Lifecycle: Application.cfc load errors");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";

if (serverPort == "" || serverPort == "0") {
    assertTrue("Application.cfc load error skipped (no cgi.server_port)", true);
} else {
    targetPath = "/tests/lifecycle/application_load_error/index.cfm";
    http url="http://127.0.0.1:#serverPort##targetPath#" method="GET" throwonerror="false" result="loadErrorResult";

    assertTrue("Application.cfc load error returns failure status",
        left(loadErrorResult.statuscode, 1) == "5");
    assertTrue("Application.cfc load error does not execute target page",
        findNoCase("direct-page-executed", loadErrorResult.filecontent) == 0);
}

suiteEnd();
</cfscript>
