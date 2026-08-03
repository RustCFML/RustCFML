<cfscript>
suiteBegin("Tags: cfdump output= / abort=");

// `<cfdump>` lowered only var/label/expand/top into the writeDump call, so
// `output=` and `abort=` were discarded at compile time: `output="console"`
// wrote the dump into the HTTP response anyway, `output="<file>"` wrote nothing
// to disk, and `abort="true"` never stopped the request (docs known-issues §27).
// Probed against Lucee 7.0.4: a console dump leaves the response untouched, a
// file `output=` writes the PLAIN-TEXT rendering and APPENDS, its path resolves
// like ExpandPath — a relative one against the BASE (request) template's
// directory, not the including file's — and `abort` emits the dump and then
// ends the request.
dumpData = { a: 1, b: "two" };
dumpDir  = getDirectoryFromPath( getBaseTemplatePath() );
dumpOut  = dumpDir & "test_cfdump_output_target.txt";
if ( fileExists( dumpOut ) ) { fileDelete( dumpOut ); }
</cfscript>

<cfsavecontent variable="dumpConsole"><cfdump var="#dumpData#" output="console"></cfsavecontent>
<cfsavecontent variable="dumpToFile"><cfdump var="#dumpData#" output="test_cfdump_output_target.txt"></cfsavecontent>
<cfsavecontent variable="dumpToBrowser"><cfdump var="#dumpData#"></cfsavecontent>

<cfscript>
assert( "cfdump output=console writes nothing to the response", len( trim( dumpConsole ) ), 0 );
assert( "cfdump output=<file> writes nothing to the response", len( trim( dumpToFile ) ), 0 );
assertTrue( "cfdump output=<file> writes the file next to the base template", fileExists( dumpOut ) );
dumpFileBody = fileExists( dumpOut ) ? fileRead( dumpOut ) : "";
assertTrue( "cfdump output=<file> file names the value's keys", dumpFileBody contains "two" );
assertFalse( "cfdump output=<file> uses the plain-text rendering, not HTML", dumpFileBody contains "<div" );
// A default dump still reaches the response.
assertTrue( "cfdump with no output= still writes to the response", len( trim( dumpToBrowser ) ) > 0 );
</cfscript>

<!--- Appends rather than truncating (Lucee-probed). --->
<cfdump var="#dumpData#" output="test_cfdump_output_target.txt">
<cfscript>
assertTrue( "cfdump output=<file> appends to an existing file",
	len( fileRead( dumpOut ) ) > len( dumpFileBody ) );
fileDelete( dumpOut );

// abort="true" — fetched over HTTP so the abort ends that request, not this one.
serverPort = structKeyExists( cgi, "server_port" ) ? trim( cgi.server_port ) : "";
if ( serverPort == "" || serverPort == "0" ) {
	assertTrue( "cfdump abort= skipped (no cgi.server_port)", true );
}
</cfscript>

<cfif serverPort NEQ "" AND serverPort NEQ "0">
	<cfhttp url="http://127.0.0.1:#serverPort#/tests/tags/cfdump_abort_target.cfm" result="abortResult">
	<cfscript>
		assertTrue( "cfdump abort= still emits the dump", abortResult.fileContent contains "before-dump" );
		assertFalse( "cfdump abort= ends the request",
			abortResult.fileContent contains "after-dump-must-not-appear" );
	</cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
