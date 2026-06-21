<cfscript>
suiteBegin("Tags: cfcookie Path and SameSite attributes");

function responseHeaderBlob(required struct result) {
    return serializeJSON(arguments.result.responseheader);
}

function statusCodeIs200(required string statusCode) {
    return left(trim(arguments.statusCode), 3) == "200";
}

function getTarget(required string testName) {
    var candidate = "";
    var lastResult = {};

    for (candidate in request.cookieBaseUrls) {
        http url=candidate & request.cookieTargetPath & "?test=" & arguments.testName method="GET" result="httpResult";
        lastResult = httpResult;
        if (statusCodeIs200(httpResult.statuscode)) {
            return httpResult;
        }
    }

    return lastResult;
}

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
if (serverPort == "" || serverPort == "0") {
    writeOutput(chr(10) & "  skipped cfcookie HTTP header subtests (no cgi.server_port - run via rustcfml --serve)" & chr(10));
} else {
    request.cookieBaseUrls = [
        "http://127.0.0.1:" & serverPort,
        "http://127.0.0.1",
        "http://localhost:" & serverPort,
        "http://localhost"
    ];
    request.cookieTargetPath = "/tests/tags/cfcookie_path_samesite_target.cfm";

    defaultResult = getTarget("default");
    assertTrue("default target responds", statusCodeIs200(defaultResult.statuscode));
    assert("default body", trim(defaultResult.filecontent), "default-ok");
    defaultHeaders = responseHeaderBlob(defaultResult);
    assertTrue("default cfcookie emits Path=/",
        findNoCase("ck_default=v", defaultHeaders) > 0
        && findNoCase("Path=/", defaultHeaders) > 0);
    assertTrue("default cfcookie emits SameSite=Lax",
        findNoCase("ck_default=v", defaultHeaders) > 0
        && findNoCase("SameSite=Lax", defaultHeaders) > 0);

    explicitResult = getTarget("explicit");
    assertTrue("explicit target responds", statusCodeIs200(explicitResult.statuscode));
    assert("explicit body", trim(explicitResult.filecontent), "explicit-ok");
    explicitHeaders = responseHeaderBlob(explicitResult);
    assertTrue("explicit cfcookie preserves path",
        findNoCase("ck_explicit=v", explicitHeaders) > 0
        && findNoCase("Path=/custom", explicitHeaders) > 0);
    assertTrue("explicit cfcookie emits SameSite=Strict",
        findNoCase("ck_explicit=v", explicitHeaders) > 0
        && findNoCase("SameSite=Strict", explicitHeaders) > 0);
    assertTrue("explicit cfcookie still emits Secure and HttpOnly",
        findNoCase("Secure", explicitHeaders) > 0
        && findNoCase("HttpOnly", explicitHeaders) > 0);

    omittedResult = getTarget("omitted");
    assertTrue("omitted target responds", statusCodeIs200(omittedResult.statuscode));
    assert("omitted body", trim(omittedResult.filecontent), "omitted-ok");
    omittedHeaders = responseHeaderBlob(omittedResult);
    assertTrue("omitted samesite does not invent SameSite",
        findNoCase("ck_omitted=v", omittedHeaders) > 0
        && findNoCase("SameSite", omittedHeaders) == 0);
}

suiteEnd();
</cfscript>
