<cfscript>
// A request that hands the upload to a cfthread which may outlive it: its
// temp file must NOT be deleted at request end (GH #386).
f = form.upload;
thread name="uploadReader" action="run" path=f.tempFilePath {
	sleep( 500 );
	fileReadBinary( attributes.path );
}
writeOutput( "tempFilePath=" & f.tempFilePath & ";" );
</cfscript>
