<cfscript>
// GH #547: several files under one field name must all reach FileUploadAll,
// and form.docs is the comma-joined temp paths.
writeOutput( "isSimpleValue=" & isSimpleValue( form.docs ) & ";" );
writeOutput( "formDocs=" & form.docs & ";" );

pathList = listToArray( form.docs );
writeOutput( "pathCount=" & arrayLen( pathList ) & ";" );
for ( i = 1; i <= arrayLen( pathList ); i++ ) {
	writeOutput( "path" & i & "Exists=" & fileExists( pathList[ i ] ) & ";" );
}

dest = getTempDirectory() & "upload_all_" & createUUID();
directoryCreate( dest );
results = fileUploadAll( dest, "", "makeunique" );
writeOutput( "uploadAllCount=" & arrayLen( results ) & ";" );
for ( i = 1; i <= arrayLen( results ); i++ ) {
	writeOutput( "clientFile" & i & "=" & results[ i ].clientFile & ";" );
}
</cfscript>
