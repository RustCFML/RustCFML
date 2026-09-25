<cfscript>
suiteBegin( "expandPath of an absolute filesystem path (Lucee 7.1 verified)" );
// Lucee returns an absolute path that EXISTS unchanged (with `..` resolved) and
// treats one that does not exist as web-root relative. Modules locate their own
// files this way: expandPath( getDirectoryFromPath( getCurrentTemplatePath() ) & "../lib/x.jar" ).
_epHere = getDirectoryFromPath( getCurrentTemplatePath() );
assert( "an existing absolute file is returned as-is", expandPath( _epHere & "test_expandpath_absolute_existing.cfm" ), _epHere & "test_expandpath_absolute_existing.cfm" );
assert( "`..` in an existing absolute path is resolved", expandPath( _epHere & "../core/test_expandpath_absolute_existing.cfm" ), _epHere & "test_expandpath_absolute_existing.cfm" );
assert( "an existing absolute directory keeps its trailing slash", expandPath( _epHere ), _epHere );
// "/" exists on disk too — but the web-root resolution exists, and wins.
assertTrue( "expandPath(""/"") is the web root, not the filesystem root", expandPath( "/" ) != "/" && directoryExists( expandPath( "/" ) ) );
assert( "expandPath(""/"") agrees with the web root of this test", expandPath( "/core/test_expandpath_absolute_existing.cfm" ), _epHere & "test_expandpath_absolute_existing.cfm" );
assertTrue( "a non-existent absolute path is web-root relative", expandPath( "/definitely/not/a/real/dir/x.jar" ) != "/definitely/not/a/real/dir/x.jar" );
suiteEnd();
</cfscript>
