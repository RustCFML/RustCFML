<cfscript>
// GH #424. A response header may legitimately repeat — Set-Cookie, Link, Vary,
// WWW-Authenticate. cfhttp exposed only the FIRST value, so a page setting two
// cookies showed the caller one and dropped the other silently, and there was
// no raw `header` key to fall back on.
//
// Lucee 7.1.0.204, measured: repeats come back as an ARRAY under the single
// key, a lone header stays a SCALAR, and `header` carries every line.
suiteBegin("cfhttp surfaces repeated response headers");

port = structKeyExists(cgi, "server_port") ? cgi.server_port : "";
if (len(port) && port != "0") {
	http result="r" url="http://127.0.0.1:#port#/tests/tags/repeated_header_target.cfm" method="get";

	assertTrue("a repeated header is an array", isArray(r.responseHeader["X-Multi"]));
	assert("both values are present", arrayLen(r.responseHeader["X-Multi"]), 2);
	assert("first value kept", r.responseHeader["X-Multi"][1], "first");
	assert("second value kept — this is the one that used to vanish",
		r.responseHeader["X-Multi"][2], "second");

	// A single-valued header must NOT become a one-element array.
	assertFalse("a lone header stays scalar", isArray(r.responseHeader["X-Single"]));
	assert("lone header value", r.responseHeader["X-Single"], "only");

	// The raw header string is the fallback callers reach for.
	assertTrue("raw header key exists", structKeyExists(r, "header"));
	assertTrue("raw header carries the repeated value",
		findNoCase("second", r.header) GT 0);
}

suiteEnd();
</cfscript>
