<cfscript>
// A component's `static` scope lives for the application's lifetime (Lucee/ACF),
// not for one request. Before v0.660.0 it was held on the per-request VM, so
// `static { hits = 0 }` read 1 on every request. Needs a live server to observe
// (three separate requests), so this skips under the CLI runner.
suiteBegin( "Component static scope persists across requests" );
serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";
if ( serverPort == "" || serverPort == "0" ) {
	assertTrue( "skipped: needs cgi.server_port (run via --serve)", true );
} else {
	base = "http://127.0.0.1:" & serverPort & "/tests/oop/static_across_requests_target.cfm";
	http url=base method="GET" result="r1";
	http url=base method="GET" result="r2";
	http url=base method="GET" result="r3";
	a = val( trim( r1.filecontent ) ); b = val( trim( r2.filecontent ) ); c = val( trim( r3.filecontent ) );
	assertTrue( "target answers", a GT 0 );
	assert( "second request continues the count", b, a + 1 );
	assert( "third request continues the count", c, a + 2 );
}
suiteEnd();
</cfscript>
