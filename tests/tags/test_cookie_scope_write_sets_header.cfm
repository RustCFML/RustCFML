<cfscript>
// GH #423. Writing to the `cookie` scope must emit a Set-Cookie header, the
// same as <cfcookie>. We updated the in-request scope only, so the value was
// readable for the rest of the request and never reached the browser — the
// reporter's cookie "created a structure" and did nothing else.
//
// This suite asserts the SCOPE side, which is what a CLI run can see. The
// header side needs a live response, so it is driven over HTTP by
// tests/tags/cookie_scope_target.cfm below when a server is available, exactly
// as the other HTTP-dependent suites do (no hardcoded port — cgi.server_port).
suiteBegin("Cookie scope writes");

// Both forms must land in the scope and read back.
cookie.plaincookie = "plainvalue";
assert("scalar write is readable in-request", cookie.plaincookie, "plainvalue");

cookie.structcookie = {value="structvalue", expires="never", httponly=true, path="/"};
assertTrue("struct write is present in the scope", structKeyExists(cookie, "structcookie"));

// Over HTTP the header is the point. Skipped on a CLI run with no server.
port = structKeyExists(cgi, "server_port") ? cgi.server_port : "";
if (len(port) && port != "0") {
	http result="r" url="http://127.0.0.1:#port#/tests/tags/cookie_scope_target.cfm" method="get";
	// `responseHeader` keeps only ONE entry per header name, so repeated
	// Set-Cookie values collapse and only the first is visible there. The raw
	// `header` string carries every line, which is what these assertions need.
	headerText = (structKeyExists(r, "header") ? r.header : "")
		& (isStruct(r.responseHeader) ? serializeJSON(r.responseHeader) : "");

	assertTrue("a cookie-scope write emits Set-Cookie",
		findNoCase("scopestruct", headerText) GT 0);
	// expires="never" must produce an Expires attribute — without it the cookie
	// dies with the browser session, which is the visible half of the bug.
	assertTrue("expires=never produces an Expires attribute",
		findNoCase("expires", headerText) GT 0);
	// An unchanged incoming cookie must NOT be echoed back as a Set-Cookie.
	assertFalse("an untouched incoming cookie is not re-emitted",
		findNoCase("untouched", headerText) GT 0);
}

suiteEnd();
</cfscript>
