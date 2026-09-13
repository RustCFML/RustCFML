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
	// Repeated Set-Cookie headers come back as an array under the one key, and
	// the raw `header` string carries every line (GH #424) — so both cookies
	// this target sets are visible here.
	headerText = (structKeyExists(r, "header") ? r.header : "")
		& (isStruct(r.responseHeader) ? serializeJSON(r.responseHeader) : "");

	assertTrue("a scalar cookie-scope write emits Set-Cookie",
		findNoCase("scopescalar", headerText) GT 0);
	assertTrue("a struct cookie-scope write emits Set-Cookie",
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
