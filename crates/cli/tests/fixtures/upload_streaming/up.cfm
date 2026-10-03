<cfscript>
// Echo just enough of the upload for the harness to verify it end to end:
// what the client called it, where it landed, how big it is, and what it holds
// (the temp file is deleted when the request ends, so the harness cannot read
// it afterwards — GH #386).
f = form.upload;
writeOutput( "clientFile=" & f.clientFile & ";" );
writeOutput( "tempFilePath=" & f.tempFilePath & ";" );
writeOutput( "fileSize=" & f.fileSize & ";" );
writeOutput( "sha256=" & lCase( hash( fileReadBinary( f.tempFilePath ), "SHA-256" ) ) & ";" );
writeOutput( "note=" & ( structKeyExists( form, "note" ) ? form.note : "" ) & ";" );
writeOutput( "rawContentLen=" & len( getHttpRequestData().content ) & ";" );
</cfscript>
