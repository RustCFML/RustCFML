<cfscript>
suiteBegin("cfhttp");

// ============================================================
// Remote HTTP server.
//
// These tests exercise <cfhttp> against a live HTTP endpoint. We use the
// RustCFML Cloudflare Worker (https://rustcfml-worker.rustcfml.workers.dev),
// which hosts a small "echo" service under /echo/ purpose-built for this
// suite. It replaces the previous httpbin.org dependency, which was flaky
// (rate limits / downtime) and routinely produced false-red runs. The worker
// runs on Cloudflare's edge, so it is effectively always-on and low latency.
//
//   /echo/request.cfm           -> JSON mirror of the request
//                                  {method,url,path,queryString,args,form,
//                                   headers,cookies,body,userAgent}
//   /echo/response-headers.cfm  -> echoes each ?param back as a response header
//   /echo/status.cfm?code=NNN   -> responds with the given status code
//
// The endpoints are generic (no engine-specific behaviour), so this file
// passes identically on RustCFML and Lucee.
// ============================================================
echoBase = "https://rustcfml-worker.rustcfml.workers.dev/echo";

// Reachability guard: if the worker can't be reached (offline dev box, edge
// outage), skip the network assertions with one informational pass rather
// than spraying ~30 false reds. A non-200 here means "can't test", not "bug".
http url="#echoBase#/request.cfm" method="GET" result="probeResult" timeout="20";
echoReachable = isStruct(probeResult) && (probeResult.status_code ?: 0) == 200;
</cfscript>

<cfif NOT echoReachable>
<cfscript>
    assertTrue("cfhttp echo server unreachable — network tests skipped", true);
    suiteEnd();
</cfscript>
<cfabort>
</cfif>

<!--- Basic GET request (tag form, via the tag preprocessor) --->
<cfhttp url="#echoBase#/request.cfm" method="GET" result="getResult">

<cfscript>
assertTrue("GET result is struct", isStruct(getResult));
assertTrue("GET status_code is 200", getResult.status_code == 200);
assertTrue("GET statusCode contains 200", find("200", getResult.statusCode) > 0);
assertTrue("GET fileContent is not empty", len(getResult.fileContent) > 0);
assertTrue("GET responseHeader is struct", isStruct(getResult.responseHeader));
// ACF/Lucee expose the status inside responseHeader too (Preside's
// ElasticSearchApiWrapper reads result.responseHeader.status_code).
assertTrue("GET responseHeader.status_code is 200", getResult.responseHeader.status_code == 200);
assertTrue("GET responseHeader.explanation present", len(getResult.responseHeader.explanation ?: "") > 0);
assertTrue("GET mimeType is json", find("json", getResult.mimeType) > 0);

// Parse the JSON response
getData = deserializeJSON(getResult.fileContent);
assertTrue("GET response has url key", structKeyExists(getData, "url"));
assert("GET response method", getData.method, "GET");
assertTrue("GET response url points at request.cfm", getData.url contains "/echo/request.cfm");

// ============================================================
// CFScript http statement syntax (semicolon form)
// ============================================================
http url="#echoBase#/request.cfm" method="GET" result="scriptResult";

assertTrue("CFScript http result is struct", isStruct(scriptResult));
assertTrue("CFScript http status 200", scriptResult.status_code == 200);
assertTrue("CFScript http fileContent not empty", len(scriptResult.fileContent) > 0);

// ============================================================
// CFScript http block form with httpparam (header + url param)
// ============================================================
http url="#echoBase#/request.cfm" method="GET" result="paramResult" {
    httpparam type="header" name="X-Custom-Header" value="TestValue123";
    httpparam type="url" name="foo" value="bar";
}

assertTrue("httpparam result is struct", isStruct(paramResult));
assertTrue("httpparam status 200", paramResult.status_code == 200);
paramData = deserializeJSON(paramResult.fileContent);
assertTrue("httpparam header sent", structKeyExists(paramData.headers, "X-Custom-Header"));
assert("httpparam header value", paramData.headers["X-Custom-Header"], "TestValue123");
assert("httpparam url param sent", paramData.args.foo, "bar");

// ============================================================
// POST with body
// ============================================================
http url="#echoBase#/request.cfm" method="POST" result="postResult" {
    httpparam type="header" name="Content-Type" value="application/json";
    httpparam type="body" value='{"name":"test","value":42}';
}

assertTrue("POST status 200", postResult.status_code == 200);
postData = deserializeJSON(postResult.fileContent);
assert("POST method echoed", postData.method, "POST");
assertTrue("POST body received", len(postData.body) > 0);
postedJson = deserializeJSON(postData.body);
assert("POST body name", postedJson.name, "test");
assert("POST body value", postedJson.value, 42);

// ============================================================
// POST with formfields
// ============================================================
http url="#echoBase#/request.cfm" method="POST" result="formResult" {
    httpparam type="formfield" name="username" value="testuser";
    httpparam type="formfield" name="email" value="test@example.com";
}

assertTrue("formfield POST status 200", formResult.status_code == 200);
formData = deserializeJSON(formResult.fileContent);
assertTrue("formfield has form key", structKeyExists(formData, "form"));
assert("formfield username", formData.form.username, "testuser");
assert("formfield email", formData.form.email, "test@example.com");

// ============================================================
// Custom headers
// ============================================================
http url="#echoBase#/request.cfm" method="GET" result="headerResult" {
    httpparam type="header" name="X-Test-One" value="Alpha";
    httpparam type="header" name="X-Test-Two" value="Beta";
}

assertTrue("headers status 200", headerResult.status_code == 200);
headerData = deserializeJSON(headerResult.fileContent);
assert("custom header X-Test-One", headerData.headers["X-Test-One"], "Alpha");
assert("custom header X-Test-Two", headerData.headers["X-Test-Two"], "Beta");

// ============================================================
// PUT method
// ============================================================
http url="#echoBase#/request.cfm" method="PUT" result="putResult" {
    httpparam type="header" name="Content-Type" value="application/json";
    httpparam type="body" value='{"updated":true}';
}

assertTrue("PUT status 200", putResult.status_code == 200);
putData = deserializeJSON(putResult.fileContent);
assert("PUT method echoed", putData.method, "PUT");
assert("PUT body received", putData.body, '{"updated":true}');

// ============================================================
// DELETE method
// ============================================================
http url="#echoBase#/request.cfm" method="DELETE" result="deleteResult";

assertTrue("DELETE status 200", deleteResult.status_code == 200);

// ============================================================
// User-Agent header
// ============================================================
http url="#echoBase#/request.cfm" method="GET" result="uaResult" useragent="RustCFML-Test/1.0";

assertTrue("useragent status 200", uaResult.status_code == 200);
uaData = deserializeJSON(uaResult.fileContent);
assert("useragent value sent", uaData.userAgent, "RustCFML-Test/1.0");

// ============================================================
// Cookie via httpparam
// ============================================================
http url="#echoBase#/request.cfm" method="GET" result="cookieResult" {
    httpparam type="cookie" name="session_id" value="abc123";
    httpparam type="cookie" name="theme" value="dark";
}

assertTrue("cookie status 200", cookieResult.status_code == 200);
cookieData = deserializeJSON(cookieResult.fileContent);
assertTrue("cookie has cookies key", structKeyExists(cookieData, "cookies"));
assert("cookie session_id", cookieData.cookies.session_id, "abc123");
assert("cookie theme", cookieData.cookies.theme, "dark");

// ============================================================
// Response headers (reflector echoes ?param -> response header)
// ============================================================
http url="#echoBase#/response-headers.cfm?X-Custom-Response=HelloWorld" method="GET" result="respHeaderResult";

assertTrue("response header status 200", respHeaderResult.status_code == 200);
assertTrue("responseHeader has custom key", structKeyExists(respHeaderResult.responseHeader, "X-Custom-Response"));
assert("responseHeader custom value", respHeaderResult.responseHeader["X-Custom-Response"], "HelloWorld");

// ============================================================
// Timeout (generous timeout, should still complete)
// ============================================================
http url="#echoBase#/request.cfm" method="GET" result="timeoutResult" timeout="20";

assertTrue("timeout request succeeded", timeoutResult.status_code == 200);

// ============================================================
// PATCH method
// ============================================================
http url="#echoBase#/request.cfm" method="PATCH" result="patchResult" {
    httpparam type="header" name="Content-Type" value="application/json";
    httpparam type="body" value='{"patched":true}';
}

assertTrue("PATCH status 200", patchResult.status_code == 200);
patchData = deserializeJSON(patchResult.fileContent);
assert("PATCH body received", patchData.body, '{"patched":true}');

// ============================================================
// Explicit status code (error path)
// ============================================================
http url="#echoBase#/status.cfm?code=404" method="GET" result="statusResult";

assertTrue("status endpoint returns 404", statusResult.status_code == 404);

suiteEnd();
</cfscript>
