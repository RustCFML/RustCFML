<cfscript>
// Hands the upload to a cfthread, so the request keeps its temp file and the
// age reaper has to remove it (GH #386).
f = form.upload;
thread name="uploadReader" action="run" path=f.tempFilePath {
	fileReadBinary( attributes.path );
}
writeOutput( "tempFilePath=" & f.tempFilePath & ";" );
</cfscript>
