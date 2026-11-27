<cfscript>
// Same as upload_streaming/thread.cfm: keep the temp path past request end
// so the age reaper has something to sweep (GH #386). form.upload is the
// temp path string (GH #546).
thread name="uploadReader" action="run" path=form.upload {
	sleep( 500 );
	fileReadBinary( attributes.path );
}
writeOutput( "tempFilePath=" & form.upload & ";" );
</cfscript>
