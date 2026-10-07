<cfscript>
suiteBegin("Server: a cookie set in long form reads back as its value (GH 480)");

// ============================================================
// Background
// ============================================================
// `cookie.x = { value = "v", httpOnly = true }` is CFML's long form for setting
// a cookie with attributes. Lucee renders the Set-Cookie header from it and
// leaves the VALUE in the scope, so a later read in the same request gives the
// string. RustCFML left the struct there, so code that set a cookie in one
// component and read it in another got a struct where it expected a string:
// comparing or concatenating it then failed, or silently compared a struct.
//
// Needs a live server (the cookie scope is per request), so it discovers the
// port from cgi.server_port and skips when run from the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("cookie long-form read-back skipped (no cgi.server_port)", true);
} else {
    baseUrl = "http://127.0.0.1:" & serverPort;
    target = "/tests/server/cookie_struct_form_target.cfm";
    httpError = "";
}
</cfscript>

<cfif NOT skip>
    <cftry>
        <cfhttp url="#baseUrl##target#" method="GET" result="cookieResult" />
        <cfcatch type="any"><cfset httpError = cfcatch.message></cfcatch>
    </cftry>

    <cfscript>
    if (len(httpError)) {
        assertTrue("cookie long-form read-back request failed: " & httpError, false);
    } else {
        body = trim(cookieResult.fileContent ?: "");
        assert("the long form reads back as its value", listGetAt(body, 1, "|"), "gh480a=alpha");
        assert("a second long-form cookie reads back as its value", listGetAt(body, 2, "|"), "gh480b=beta");
        assert("the plain form is unchanged", listGetAt(body, 3, "|"), "gh480c=gamma");
        assert("the value concatenates", listGetAt(body, 4, "|"), "concat=alpha");

        // The attributes still reach the browser: the header is rendered from
        // them even though the scope holds only the value.
        headers = cookieResult.responseHeader ?: {};
        setCookie = structKeyExists(headers, "Set-Cookie") ? headers["Set-Cookie"] : "";
        allCookies = isArray(setCookie) ? arrayToList(setCookie, " ## ") : setCookie;
        assertTrue("httpOnly survives on the header",
            reFindNoCase("gh480a=alpha[^##]*HttpOnly", allCookies) gt 0);
        assertTrue("the plain-form cookie is still sent",
            reFindNoCase("gh480c=gamma", allCookies) gt 0);
    }
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
