<cfscript>
// cachePut / cacheGet are an application-lifetime cache. Before v0.660.0 the
// store lived on the per-request VM, so a value put in one request was gone in
// the next — the cache functions did nothing useful in serve mode. Needs a live
// server (separate requests); skips under the CLI runner.
suiteBegin( "cachePut/cacheGet persist across requests" );
serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";
if ( serverPort == "" || serverPort == "0" ) {
	assertTrue( "skipped: needs cgi.server_port (run via --serve)", true );
} else {
	key  = "xreq_" & createUUID();
	base = "http://127.0.0.1:" & serverPort & "/tests/stdlib/cache_across_requests_target.cfm?key=" & key & "&op=";
	http url=base & "get"    method="GET" result="r0";
	http url=base & "put"    method="GET" result="r1";
	http url=base & "get"    method="GET" result="r2";
	http url=base & "exists" method="GET" result="r3";
	http url=base & "delete" method="GET" result="r4";
	http url=base & "get"    method="GET" result="r5";
	assert( "unknown key misses",                     trim( r0.filecontent ), "MISSING" );
	assert( "value put in one request is read in the next", trim( r2.filecontent ), "value-from-put-request" );
	assert( "cacheKeyExists sees it from another request",   trim( r3.filecontent ), "yes" );
	assert( "cacheDelete in one request removes it for the next", trim( r5.filecontent ), "MISSING" );
}
suiteEnd();
</cfscript>
