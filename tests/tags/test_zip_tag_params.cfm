<cfscript>
suiteBegin("cfzip tag body: cfzipparam children");
zipDir = getTempDirectory() & "/rustcfml_zipparam_tag_test";
if ( DirectoryExists( zipDir ) ) { DirectoryDelete( zipDir, true ); }
DirectoryCreate( zipDir );
FileWrite( zipDir & "/a.txt", "AAA" );
zipFile = zipDir & "/tagform.zip";
</cfscript>
<cfzip file="#zipFile#">
	<cfzipparam source="#zipDir#/a.txt" entrypath="tagged/a.txt">
</cfzip>
<cfscript>
assertTrue( "<cfzip> with <cfzipparam> children writes the archive", FileExists( zipFile ) );
cfzip( action="list", file=zipFile, name="tagEntries" );
names = "";
for ( row in tagEntries ) { names = ListAppend( names, row.name ); }
assert( "the child tag's entrypath is honoured", names, "tagged/a.txt" );
DirectoryDelete( zipDir, true );
suiteEnd();
</cfscript>
