<cfscript>
// GH #546 / #547: form.<field> is a simple temp-path value; FileUpload returns
// the full struct; FileUploadAll returns every part under a repeated name.
writeOutput( "isSimpleValue=" & isSimpleValue( form.doc ) & ";" );
writeOutput( "fileExists=" & fileExists( form.doc ) & ";" );
writeOutput( "formDoc=" & form.doc & ";" );

dest = getTempDirectory() & "upload_form_and_all_" & createUUID();
directoryCreate( dest );
r = fileUpload( dest, "doc", "", "makeunique" );
writeOutput( "uploadClientFile=" & r.clientFile & ";" );
writeOutput( "uploadSaved=" & r.fileWasSaved & ";" );
</cfscript>
