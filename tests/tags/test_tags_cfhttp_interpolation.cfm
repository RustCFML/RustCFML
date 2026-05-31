<cfscript>
suiteBegin("Tags: cfhttp interpolation");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("tag cfhttp interpolation skipped (no cgi.server_port)", true);
} else {
    baseUrl = "http://127.0.0.1:" & serverPort;
    targetPath = "/tests/tags/http_statements_target.cfm";
    httpMethod = "GET";
    dynamicPath = targetPath & "?test=echo";
    dynamicHttpError = "";
}
</cfscript>

<cfif NOT skip>
    <cftry>
        <cfhttp url="#baseUrl##dynamicPath#" method="#httpMethod#" result="dynamicHttpResult">
        <cfcatch type="any">
            <cfset dynamicHttpError = cfcatch.message>
        </cfcatch>
    </cftry>
    <cfscript>
        assert("tag cfhttp interpolated attributes error", dynamicHttpError, "");
        assert("tag cfhttp interpolated attributes body",
            structKeyExists(variables, "dynamicHttpResult") ? trim(dynamicHttpResult.filecontent) : "",
            "echo-ok");
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
