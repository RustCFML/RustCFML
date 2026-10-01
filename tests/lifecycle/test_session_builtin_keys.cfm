<cfscript>
suiteBegin("Session scope: Lucee's built-in keys");

// Every Lucee session scope carries cfid, cftoken, sessionid, urltoken,
// timecreated and lastvisit. They are rebuilt each request, so structClear()
// removes them only until the next one. Runs only when served.

serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";

function sbCookieHeader( resp ) {
	var sc = resp.responseheader[ "Set-Cookie" ] ?: "";
	var raw = isArray( sc ) ? sc : [ sc ];
	var pairs = [];
	for ( var c in raw ) {
		var firstPart = trim( listFirst( c, ";" ) );
		if ( len( firstPart ) ) { arrayAppend( pairs, firstPart ); }
	}
	return arrayToList( pairs, "; " );
}

function sbGet( required string op, string cookieHeader="" ) {
	var r = "";
	cfhttp( url="http://127.0.0.1:#serverPort#/tests/lifecycle/session_builtins/page.cfm", result="r", timeout=15 ) {
		cfhttpparam( type="url", name="op", value=arguments.op );
		if ( len( arguments.cookieHeader ) ) {
			cfhttpparam( type="header", name="Cookie", value=arguments.cookieHeader );
		}
	}
	return r;
}

if ( serverPort == "" || serverPort == "0" ) {
	assertTrue( "session built-in keys skipped (no cgi.server_port)", true );
} else {
	r1 = sbGet( "write" );
	cookieHeader = sbCookieHeader( r1 );
	s1 = deserializeJSON( r1.filecontent );
	cfid = reReplaceNoCase( reMatchNoCase( "CFID=[^;]+", cookieHeader )[ 1 ] ?: "", "^CFID=", "" );
	assertTrue( "the session cookie carries a CFID", len( cfid ) > 0 );
	assert( "cfid is the session cookie's id", s1.cfid, cfid );
	assert( "cftoken", s1.cftoken, "0" );
	assert( "sessionid is <app>_<cfid>_0", s1.sessionid, s1.app & "_" & cfid & "_0" );
	assert( "urltoken", s1.urltoken, "CFID=" & cfid & "&CFTOKEN=0" );
	assertTrue( "timecreated is a date", s1.timecreatedIsDate );
	assertTrue( "lastvisit is a date", s1.lastvisitIsDate );

	r2 = sbGet( "read", cookieHeader );
	s2 = deserializeJSON( r2.filecontent );
	assert( "the next request: same sessionid", s2.sessionid, s1.sessionid );
	assert( "the next request: same timecreated", s2.timecreated, s1.timecreated );
	assert( "the next request keeps session data", s2.mine, "kept" );

	r3 = sbGet( "clear", cookieHeader );
	s3 = deserializeJSON( r3.filecontent );
	assert( "structClear( session ) removes them for this request", s3.keys, "" );

	r4 = sbGet( "read", cookieHeader );
	s4 = deserializeJSON( r4.filecontent );
	assert( "after a clear the next request has them again", s4.sessionid, s1.sessionid );
	assert( "the clear removed the session's data", s4.mine, "" );
}

suiteEnd();
</cfscript>
