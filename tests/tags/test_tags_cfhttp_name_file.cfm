<cfscript>
suiteBegin("Tags: cfhttp name= / file= / dropped attributes");

// <cfhttp>'s tag lowering used to copy a fixed ten-attribute whitelist into the
// options struct, discarding the rest at COMPILE time (docs known-issues §27):
//   name=          -> the response was never parsed into a query and the
//                     variable was never created
//   file= / path=  -> the response body was never written to disk
//   throwOnError=, redirect=, port=, proxyPort=, encodeURL=
//                  -> implemented in the runtime, lost in the lowering
// Every expectation below was probed against Lucee 7.0.4 first.
serverPort = structKeyExists(cgi, "server_port") ? trim(cgi.server_port) : "";
skip = serverPort == "" || serverPort == "0";

if (skip) {
    assertTrue("tag cfhttp name=/file= skipped (no cgi.server_port)", true);
} else {
    base = "http://127.0.0.1:" & serverPort & "/tests/tags/cfhttp_query_target.cfm";
    tmp  = getTempDirectory();
}
</cfscript>

<cfif NOT skip>
<cfscript>
// ---------------------------------------------------------------- name= ------
q1 = "";
</cfscript>
<cfhttp url="#base#?mode=csv" name="q1" result="r1">
<cfscript>
	assertTrue( "cfhttp name= creates a query", isQuery( q1 ) );
	assert( "cfhttp name= recordcount", q1.recordcount, 2 );
	assert( "cfhttp name= columns", listSort( ucase( q1.columnlist ), "text" ), "AGE,NAME" );
	assert( "cfhttp name= row 1 value", q1.name[1], "alice" );
	assert( "cfhttp name= row 2 value", q1.age[2], "25" );
	// The response struct is unaffected — Lucee populates fileContent as well.
	assert( "cfhttp name= still fills fileContent", left( r1.fileContent, 8 ), "name,age" );

q2 = "";
</cfscript>
<cfhttp url="#base#?mode=pipe" name="q2" delimiter="|">
<cfscript>
	assert( "cfhttp name= delimiter=| columns", listSort( ucase( q2.columnlist ), "text" ), "AGE,NAME" );
	assert( "cfhttp name= delimiter=| value", q2.name[2], "bob" );

q3 = "";
</cfscript>
<cfhttp url="#base#?mode=quoted" name="q3">
<cfscript>
	// textQualifier defaults to a double quote: the qualifier is stripped and a
	// delimiter inside a qualified field does not split it.
	assert( "cfhttp name= default textQualifier strips quotes", q3.name[1], "alice" );
	assert( "cfhttp name= default textQualifier protects delimiter", q3.note[1], "likes, commas" );

q4 = "";
</cfscript>
<cfhttp url="#base#?mode=noheader" name="q4" firstrowasheaders="false" columns="nm,ag">
<cfscript>
	assert( "cfhttp name= firstRowAsHeaders=false + columns= names", listSort( ucase( q4.columnlist ), "text" ), "AG,NM" );
	assert( "cfhttp name= firstRowAsHeaders=false + columns= recordcount", q4.recordcount, 2 );
	assert( "cfhttp name= firstRowAsHeaders=false + columns= value", q4.nm[1], "alice" );

q5 = "";
</cfscript>
<cfhttp url="#base#?mode=csv" name="q5" firstrowasheaders="false">
<cfscript>
	// No header row and no columns= — Lucee names them COLUMN_1..COLUMN_N and
	// the first line counts as data.
	assert( "cfhttp name= firstRowAsHeaders=false generated names", listSort( ucase( q5.columnlist ), "text" ), "COLUMN_1,COLUMN_2" );
	assert( "cfhttp name= firstRowAsHeaders=false keeps first line", q5.recordcount, 3 );
	assert( "cfhttp name= firstRowAsHeaders=false first row is data", q5.column_1[1], "name" );

q6 = "";
</cfscript>
<cfhttp url="#base#?mode=csv" name="q6" columns="x,y">
<cfscript>
	// columns= renames, but the header row is STILL consumed (Lucee-probed).
	assert( "cfhttp name= columns= overrides header names", listSort( ucase( q6.columnlist ), "text" ), "X,Y" );
	assert( "cfhttp name= columns= still consumes the header row", q6.recordcount, 2 );

q7 = "";
</cfscript>
<cfhttp url="#base#?mode=escaped" name="q7">
<cfscript>
	assert( "cfhttp name= doubled qualifier is a literal qualifier", q7.a[1], 'say "hi"' );

q8 = "";
</cfscript>
<cfhttp url="#base#?mode=blanks" name="q8">
<cfscript>
	assert( "cfhttp name= skips blank lines", q8.recordcount, 1 );

q9 = "";
</cfscript>
<cfhttp url="#base#?mode=zeros" name="q9">
<cfscript>
	// Cells stay strings: a numeric-looking value keeps its leading zeros.
	assert( "cfhttp name= does not coerce numeric-looking cells", q9.code[1], "007" );
	assert( "cfhttp name= keeps cell length", len( q9.code[1] ), 3 );
</cfscript>

<cftry>
	<cfhttp url="#base#?mode=ragged" name="qBad">
	<cfscript> assertTrue( "cfhttp name= ragged row must throw", false ); </cfscript>
	<cfcatch>
		<cfscript>
			assertTrue( "cfhttp name= ragged row error wording",
				cfcatch.message contains "Invalid CSV line size" );
		</cfscript>
	</cfcatch>
</cftry>

<!--- ------------------------------------------------------ file= / path= ---->
<cfscript>
	fileTarget = tmp & "test_cfhttp_name_file_a.txt";
	if ( fileExists( fileTarget ) ) { fileDelete( fileTarget ); }
</cfscript>
<cfhttp url="#base#?mode=csv" file="test_cfhttp_name_file_a.txt" path="#tmp#" result="r2">
<cfscript>
	assertTrue( "cfhttp file=+path= writes the file", fileExists( fileTarget ) );
	assert( "cfhttp file=+path= file content", trim( fileRead( fileTarget ) ), "name,age" & chr(10) & "alice,30" & chr(10) & "bob,25" );
	assert( "cfhttp file= still fills fileContent", left( r2.fileContent, 8 ), "name,age" );

	// An existing file is overwritten, not appended to.
	fileWrite( fileTarget, "PREEXISTING" );
</cfscript>
<cfhttp url="#base#?mode=csv" file="test_cfhttp_name_file_a.txt" path="#tmp#">
<cfscript>
	assertFalse( "cfhttp file= overwrites an existing file", fileRead( fileTarget ) contains "PREEXISTING" );
	fileDelete( fileTarget );

	// path= with no file= derives the leaf from the URL's last segment.
	urlNamed = tmp & "cfhttp_query_target.cfm";
	if ( fileExists( urlNamed ) ) { fileDelete( urlNamed ); }
</cfscript>
<cfhttp url="#base#?mode=csv" path="#tmp#">
<cfscript>
	assertTrue( "cfhttp path= alone names the file after the URL", fileExists( urlNamed ) );
	fileDelete( urlNamed );
</cfscript>

<cftry>
	<cfhttp url="#base#?mode=csv" file="n.txt" path="#tmp#no_such_dir_#createUUID()#/">
	<cfscript> assertTrue( "cfhttp path= to a missing directory must throw", false ); </cfscript>
	<cfcatch>
		<cfscript>
			assertTrue( "cfhttp path= missing-directory error wording",
				cfcatch.message contains "does not exist" );
		</cfscript>
	</cfcatch>
</cftry>

<!--- name= and file= together: both happen ---------------------------------->
<cfscript>
	bothTarget = tmp & "test_cfhttp_name_file_both.txt";
	if ( fileExists( bothTarget ) ) { fileDelete( bothTarget ); }
	qBoth = "";
</cfscript>
<cfhttp url="#base#?mode=csv" name="qBoth" file="test_cfhttp_name_file_both.txt" path="#tmp#">
<cfscript>
	assertTrue( "cfhttp name=+file= builds the query", isQuery( qBoth ) );
	assertTrue( "cfhttp name=+file= writes the file", fileExists( bothTarget ) );
	fileDelete( bothTarget );
</cfscript>

<!--- ------------------------------------- throwOnError / redirect / port ---->
<cftry>
	<cfhttp url="#base#?mode=notfound" throwOnError="true" result="rThrow">
	<cfscript> assertTrue( "cfhttp throwOnError=true must throw on 404", false ); </cfscript>
	<cfcatch>
		<cfscript>
			// Lucee: type=[application], message=[404 Not Found].
			assert( "cfhttp throwOnError= message", trim( cfcatch.message ), "404 Not Found" );
		</cfscript>
	</cfcatch>
</cftry>

<cfhttp url="#base#?mode=notfound" result="rQuiet">
<cfscript>
	assertTrue( "cfhttp default throwOnError does not throw on 404", rQuiet.statusCode contains "404" );
	assert( "cfhttp 404 body still available", trim( rQuiet.fileContent ), "nope" );
</cfscript>

<cfhttp url="#base#?mode=redirect" redirect="false" result="rNoRedirect">
<cfscript>
	assertTrue( "cfhttp redirect=false stops at the 302", rNoRedirect.statusCode contains "302" );
	assertTrue( "cfhttp redirect=false exposes the location header",
		( rNoRedirect.responseHeader.location ?: "" ) contains "mode=csv" );
</cfscript>

<cfhttp url="#base#?mode=redirect" result="rRedirect">
<cfscript>
	assertTrue( "cfhttp default follows the redirect", rRedirect.statusCode contains "200" );
	assert( "cfhttp default redirect lands on the target", left( trim( rRedirect.fileContent ), 8 ), "name,age" );
</cfscript>

<cfhttp url="http://127.0.0.1/tests/tags/cfhttp_query_target.cfm?mode=csv" port="#serverPort#" result="rPort">
<cfscript>
	assertTrue( "cfhttp port= is applied to a portless URL", rPort.statusCode contains "200" );
	assert( "cfhttp port= reached the target", left( trim( rPort.fileContent ), 8 ), "name,age" );
</cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
