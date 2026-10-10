<cfscript>
suiteBegin("Server: a long-form cookie overwritten with a plain value sends the LAST value (GH 486)");

// ============================================================
// Background
// ============================================================
// `cookie.x = { value = …, httpOnly = … }` parks its attributes beside the
// collapsed value so the response can be rendered from them (GH #480). A later
// PLAIN write — `cookie.x = "second"` — replaced the value without going near
// that attribute store, and the response was still rendered from the stale
// struct: the browser was sent the FIRST value and the application's write was
// silently dropped.
//
// Lucee emits BOTH headers, in order, which leaves the browser on the second
// value. One header carrying the final value leaves it in the same state, so
// the contract asserted here is what the browser ends up holding: the LAST
// Set-Cookie naming a cookie carries the last value written.
//
// Needs a live server (the cookie scope is per request), so it discovers the
// port from cgi.server_port and skips when run from the CLI.
// ============================================================

serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("cookie overwrite skipped (no cgi.server_port)", true);
} else {
    baseUrl = "http://127.0.0.1:" & serverPort;
    target = "/tests/server/cookie_overwrite_target.cfm";
    httpError = "";
}

// The value the browser keeps is the one on the LAST header naming the cookie,
// whether the engine sent one header or every write.
function gh486LastValue( required string headers, required string name ) {
    var found = "";
    for ( var h in listToArray( arguments.headers, chr(30) ) ) {
        var bits = reMatchNoCase( arguments.name & "=([^;]*)", trim( h ) );
        if ( arrayLen( bits ) ) {
            found = listRest( bits[ arrayLen( bits ) ], "=" );
        }
    }
    return found;
}
</cfscript>

<cfif NOT skip>
    <cftry>
        <cfhttp url="#baseUrl##target#" method="GET" result="ovResult" />
        <cfcatch type="any"><cfset httpError = cfcatch.message></cfcatch>
    </cftry>

    <cfscript>
    if (len(httpError)) {
        assertTrue("cookie overwrite request failed: " & httpError, false);
    } else {
        headers = ovResult.responseHeader ?: {};
        setCookie = structKeyExists(headers, "Set-Cookie") ? headers["Set-Cookie"] : "";
        // chr(30) keeps the joined headers separable without colliding with the
        // ';' and ',' that appear inside cookie attributes.
        allCookies = isArray(setCookie) ? arrayToList(setCookie, chr(30)) : setCookie;

        assert("a plain write after a long-form write is what the browser keeps",
            gh486LastValue( allCookies, "gh486a" ), "second");
        assert("two plain writes are unchanged",
            gh486LastValue( allCookies, "gh486b" ), "second");
        // Delete-then-reset is ours alone. Lucee sends THREE headers for such a
        // cookie — the original, the reset, and then an EXPIRY dated 1970 — so
        // the browser is left with the cookie DELETED even though Lucee's own
        // cookie scope reads back "reset" at the end of the request. Copying a
        // response that contradicts the scope that produced it would be the
        // wrong parity to buy, so we send the value the scope actually holds.
        // (Measured on Lucee 7.1.0.204.)
        if ( isRustCFML() ) {
            assert("a deleted long-form cookie does not resurrect its old value",
                gh486LastValue( allCookies, "gh486c" ), "reset");
        }
        assert("a long form left alone still sends its value",
            gh486LastValue( allCookies, "gh486d" ), "kept");

        // The overwritten cookie is sent plainly: the attributes belonged to the
        // value that was replaced, so carrying httpOnly/Path=/ over to the new
        // one would be inventing a write the application never made.
        lastA = "";
        for ( h in listToArray( allCookies, chr(30) ) ) {
            if ( reFindNoCase( "gh486a=", h ) ) { lastA = trim( h ); }
        }
        assert("the replacing write does not inherit the replaced one's attributes",
            reFindNoCase( "HttpOnly", lastA ) GT 0, false);

        // The untouched long form keeps every attribute it declared.
        assertTrue("an untouched long form still carries httpOnly",
            reFindNoCase( "gh486d=kept[^" & chr(30) & "]*HttpOnly", allCookies ) GT 0);
        assertTrue("an untouched long form still carries its path",
            reFindNoCase( "gh486d=kept[^" & chr(30) & "]*Path=/deep", allCookies ) GT 0);
    }
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
