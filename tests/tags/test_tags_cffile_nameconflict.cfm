<cfscript>
suiteBegin("Tags: cffile nameConflict on copy/move");

// `<cffile action="copy"/"move">` lowered to fileCopy/fileMove with only source
// and destination, so `nameConflict=` was discarded at compile time and every
// conflict silently OVERWROTE the destination (docs known-issues §27).
// Probed against Lucee 7.0.4: overwrite is the default, skip leaves the
// destination alone without erroring, error throws `application` "Destination
// file [x] already exists", and makeunique leaves the destination alone and
// writes `name-<unique>.ext` alongside it.
ncDir = getTempDirectory() & "rcf_cffile_nc_" & createUUID() & "/";
directoryCreate( ncDir );

function ncReset() {
	fileWrite( ncDir & "src.txt", "SOURCE" );
	fileWrite( ncDir & "dst.txt", "EXISTING" );
}
function ncNewFiles() {
	// Files created beside dst.txt by makeunique.
	return arrayLen( directoryList( ncDir, false, "name", "dst-*.txt" ) );
}
ncReset();
</cfscript>

<cffile action="copy" source="#ncDir#src.txt" destination="#ncDir#dst.txt" nameConflict="skip">
<cfscript>
assert( "cffile copy nameConflict=skip leaves the destination", fileRead( ncDir & "dst.txt" ), "EXISTING" );
assert( "cffile copy nameConflict=skip creates nothing", ncNewFiles(), 0 );

ncErr = "";
</cfscript>
<cftry>
	<cffile action="copy" source="#ncDir#src.txt" destination="#ncDir#dst.txt" nameConflict="error">
	<cfcatch><cfscript> ncErr = cfcatch.message; </cfscript></cfcatch>
</cftry>
<cfscript>
assertTrue( "cffile copy nameConflict=error throws", ncErr contains "already exists" );
assert( "cffile copy nameConflict=error leaves the destination", fileRead( ncDir & "dst.txt" ), "EXISTING" );
</cfscript>

<cffile action="copy" source="#ncDir#src.txt" destination="#ncDir#dst.txt" nameConflict="makeunique">
<cfscript>
assert( "cffile copy nameConflict=makeunique leaves the destination", fileRead( ncDir & "dst.txt" ), "EXISTING" );
assert( "cffile copy nameConflict=makeunique writes a second file", ncNewFiles(), 1 );
</cfscript>

<cffile action="copy" source="#ncDir#src.txt" destination="#ncDir#dst.txt" nameConflict="overwrite">
<cfscript>
assert( "cffile copy nameConflict=overwrite replaces the destination", fileRead( ncDir & "dst.txt" ), "SOURCE" );
ncReset();
</cfscript>

<cffile action="copy" source="#ncDir#src.txt" destination="#ncDir#dst.txt">
<cfscript>
assert( "cffile copy defaults to overwrite", fileRead( ncDir & "dst.txt" ), "SOURCE" );

// move: the destination is untouched, the source still goes away, and the
// unique name appears next to it.
fileWrite( ncDir & "mv.txt", "MOVED" );
fileWrite( ncDir & "mvdst.txt", "EXISTING3" );
</cfscript>

<cffile action="move" source="#ncDir#mv.txt" destination="#ncDir#mvdst.txt" nameConflict="makeunique">
<cfscript>
assert( "cffile move nameConflict=makeunique leaves the destination", fileRead( ncDir & "mvdst.txt" ), "EXISTING3" );
assertFalse( "cffile move nameConflict=makeunique still moves the source", fileExists( ncDir & "mv.txt" ) );
assert( "cffile move nameConflict=makeunique writes the unique name", arrayLen( directoryList( ncDir, false, "name", "mvdst-*.txt" ) ), 1 );

directoryDelete( ncDir, true );
suiteEnd();
</cfscript>
