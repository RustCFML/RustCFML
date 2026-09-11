<cfscript>
// A class's construction tables (method tables, `super` struct, blueprint) are
// built on its first construction in a request and, since v0.661.0, kept on the
// server so the next request's FIRST construction replays them too. Nothing
// observable may differ between the request that built them and the ones that
// adopt them. Needs a live server (three separate requests), so this skips
// under the CLI runner.
suiteBegin( "Component construction tables shared across requests" );
serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";
if ( serverPort == "" || serverPort == "0" ) {
	assertTrue( "skipped: needs cgi.server_port (run via --serve)", true );
} else {
	base = "http://127.0.0.1:" & serverPort & "/tests/oop/class_cache_across_requests_target.cfm";
	http url=base method="GET" result="r1";
	http url=base method="GET" result="r2";
	http url=base method="GET" result="r3";
	a = trim( r1.filecontent ); b = trim( r2.filecontent ); c = trim( r3.filecontent );
	parts = listToArray( a, "|", true );
	assert( "target answers with every field", arrayLen( parts ), 7 );
	assert( "every pseudo-constructor ran once, root first", parts[ 1 ], "Root,Mid,Leaf" );
	assert( "super dispatch through the chain", parts[ 2 ], "mid-method+mid-override+root-method" );
	assert( "override wins, super reaches the parent", parts[ 3 ], "mid-override+root-method" );
	assert( "root constructor's private write visible", parts[ 4 ], "root" );
	assert( "leaf metadata lists only own functions", parts[ 6 ], "inheritedOnThis,inheritedRefWorks,init,leafMethod,ownKeys,seenFromRoot,thisKeysInCtor,viaSuper" );
	assert( "second request (adopted tables) identical", b, a );
	assert( "third request identical", c, a );
}
suiteEnd();
</cfscript>
