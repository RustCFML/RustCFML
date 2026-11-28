<cfscript>
// GH #547: FileUpload on a multi-part field takes the first part (Lucee).
writeOutput( "pathCount=" & listLen( form.docs ) & ";" );
dest = getTempDirectory() & "upload_first_" & createUUID();
directoryCreate( dest );
r = fileUpload( dest, "docs", "", "makeunique" );
writeOutput( "firstClientFile=" & r.clientFile & ";" );
writeOutput( "saved=" & r.fileWasSaved & ";" );
</cfscript>
