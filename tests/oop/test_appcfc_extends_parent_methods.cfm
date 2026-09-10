<cfscript>
// An Application.cfc that extends a parent (Preside: `extends="preside.system.Bootstrap"`)
// must see the parent's methods — private helpers included — from its lifecycle
// methods. v0.659.0's construction replay broke this: the Application.cfc path
// resolves its parent a second time in the request, that replayed template kept
// its methods in a table the merge did not read, and every Bootstrap helper was
// "undefined" from onRequestStart. Needs a real request into a directory with
// its own Application.cfc, so this skips under the CLI runner.
suiteBegin( "Application.cfc extending a parent sees the parent's methods" );
serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";
if ( serverPort == "" || serverPort == "0" ) {
	assertTrue( "skipped: needs cgi.server_port (run via --serve)", true );
} else {
	base = "http://127.0.0.1:" & serverPort & "/tests/appcfc_parent/index.cfm";
	http url=base method="GET" result="r1";
	http url=base method="GET" result="r2";
	assert( "first request: parent private method callable from onRequestStart", listFirst( trim( r1.filecontent ), "|" ), "pong" );
	assert( "second request (parent replayed): still callable", listFirst( trim( r2.filecontent ), "|" ), "pong" );
	assertTrue( "variables lists the parent's private helper", listFindNoCase( listLast( trim( r2.filecontent ), "|" ), "_ping" ) GT 0 );
}
suiteEnd();
</cfscript>
