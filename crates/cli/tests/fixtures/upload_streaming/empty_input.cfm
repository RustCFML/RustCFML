<cfscript>
// A file input left empty (filename="" and no body) is no upload, as on Lucee:
// form.<field> is "", FileUpload on it throws, FileUploadAll saves nothing.
writeOutput( "formDocs=[" & form.docs & "];" );

dest = getTempDirectory() & "upload_empty_" & createUUID();
directoryCreate( dest );
try {
	fileUpload( dest, "docs", "", "makeunique" );
	writeOutput( "uploadThrew=false;" );
} catch ( any e ) {
	writeOutput( "uploadThrew=true;" );
}
writeOutput( "uploadAllCount=" & arrayLen( fileUploadAll( dest, "", "makeunique" ) ) & ";" );
writeOutput( "savedFiles=" & arrayLen( directoryList( dest ) ) & ";" );
directoryDelete( dest, true );
</cfscript>
