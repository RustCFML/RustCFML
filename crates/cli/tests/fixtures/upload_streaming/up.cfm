<cfscript>
// Echo just enough of the upload for the harness to verify it end to end:
// what the client called it, where it landed, and how big it is.
f = form.upload;
writeOutput( "clientFile=" & f.clientFile & ";" );
writeOutput( "tempFilePath=" & f.tempFilePath & ";" );
writeOutput( "fileSize=" & f.fileSize & ";" );
writeOutput( "note=" & ( structKeyExists( form, "note" ) ? form.note : "" ) & ";" );
writeOutput( "rawContentLen=" & len( getHttpRequestData().content ) & ";" );
</cfscript>
