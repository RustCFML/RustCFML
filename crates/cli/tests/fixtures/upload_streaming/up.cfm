<cfscript>
// Echo just enough of the upload for the harness to verify it end to end:
// what the client called it, where it landed, how big it is, and what it holds
// (the temp file is deleted when the request ends, so the harness cannot read
// it afterwards — GH #386).
//
// form.<field> is the temp path string (Lucee/ACF parity; GH #546). Metadata
// comes from FileUpload, which reads the per-request upload registry.
tempPath = form.upload;
writeOutput( "isSimpleValue=" & isSimpleValue( form.upload ) & ";" );
writeOutput( "tempFilePath=" & tempPath & ";" );
writeOutput( "fileExists=" & fileExists( tempPath ) & ";" );
writeOutput( "fileSize=" & getFileInfo( tempPath ).size & ";" );
writeOutput( "sha256=" & lCase( hash( fileReadBinary( tempPath ), "SHA-256" ) ) & ";" );
dest = getTempDirectory() & "upload_streaming_echo_" & createUUID();
directoryCreate( dest );
r = fileUpload( dest, "upload", "", "makeunique" );
writeOutput( "clientFile=" & r.clientFile & ";" );
writeOutput( "note=" & ( structKeyExists( form, "note" ) ? form.note : "" ) & ";" );
writeOutput( "rawContentLen=" & len( getHttpRequestData().content ) & ";" );
</cfscript>
