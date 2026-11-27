<cfscript>
// A request that hands the upload to a cfthread which may outlive it: its
// temp file must NOT be deleted at request end (GH #386).
// form.upload is the temp path string (GH #546).
thread name="uploadReader" action="run" path=form.upload {
	sleep( 500 );
	fileReadBinary( attributes.path );
}
writeOutput( "tempFilePath=" & form.upload & ";" );
</cfscript>
