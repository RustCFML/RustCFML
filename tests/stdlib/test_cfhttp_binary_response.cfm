<cfscript>
suiteBegin("cfhttp binary response");

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("cfhttp binary response skipped (no cgi.server_port)", true);
} else {
    targetPath = "/tests/stdlib/cfhttp_binary_target.cfm";
    targetUrl = "http://127.0.0.1:" & serverPort & targetPath;
    fallbackTargetUrl = "http://127.0.0.1" & targetPath;
}
</cfscript>

<cfif NOT skip>
    <cfhttp url="#targetUrl#" method="GET" result="binaryResult" getAsBinary="yes">

    <cfif binaryResult.status_code NEQ 200>
        <cfhttp url="#fallbackTargetUrl#" method="GET" result="binaryResult" getAsBinary="yes">
    </cfif>

    <cfscript>
    assertTrue("getAsBinary returns binary fileContent", isBinary(binaryResult.fileContent));
    assert("getAsBinary preserves exact bytes", binaryEncode(binaryResult.fileContent, "hex"), "00FF10414280");
    assert("binary response status", binaryResult.status_code, 200);
    assert("binary response mime type", binaryResult.mimeType, "application/octet-stream");
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
