<cfscript>
suiteBegin("Tags: cfhttp attributeCollection");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("cfhttp attributeCollection skipped (no cgi.server_port)", true);
} else {
    target = "http://127.0.0.1:" & serverPort & "/tests/tags/http_statements_target.cfm?test=echo";
    httpAttrs = {
        url = target,
        method = "GET",
        timeout = 15
    };
    httpError = "";
}
</cfscript>

<cfif NOT skip>
    <cftry>
        <cfhttp attributeCollection="#httpAttrs#" result="httpResult" />
        <cfcatch type="any">
            <cfset httpError = cfcatch.message />
        </cfcatch>
    </cftry>

    <cfscript>
    assert("cfhttp attributeCollection url/method error", httpError, "");
    assert("cfhttp attributeCollection url/method status",
        structKeyExists(variables, "httpResult") ? httpResult.statuscode : "",
        "200 OK");
    assert("cfhttp attributeCollection url/method body",
        structKeyExists(variables, "httpResult") ? trim(httpResult.filecontent) : "",
        "echo-ok");
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
