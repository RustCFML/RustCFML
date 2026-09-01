<cfscript>
suiteBegin("Script-form child tags: cfzipparam / cfinvokeArgument");

// A body tag written in script carries its child tags as parenthesised calls.
// Neither of these parsed, and because a `{ ... }` after a call reads as a
// struct literal, the failure was a syntax error that took the WHOLE component
// down — two Preside extensions ship exactly these shapes.

// --- cfzip { cfzipparam } ---------------------------------------------------
zipDir = getTempDirectory() & "/rustcfml_zipparam_test";
if ( DirectoryExists( zipDir ) ) { DirectoryDelete( zipDir, true ); }
DirectoryCreate( zipDir );
FileWrite( zipDir & "/a.txt", "AAA" );
FileWrite( zipDir & "/b.txt", "BBB" );
zipFile = zipDir & "/out.zip";

includeSecond = true;
cfzip( file = zipFile ) {
	cfzipparam( source = zipDir & "/a.txt" );
	// A param inside control flow must still be collected — the reason params
	// are gathered at runtime rather than harvested at parse time.
	if ( includeSecond ) {
		cfzipparam( source = zipDir & "/b.txt", entrypath = "nested/b.txt" );
	}
}
assertTrue( "cfzip( file=… ) { cfzipparam(…) } writes the archive", FileExists( zipFile ) );

cfzip( action="list", file=zipFile, name="zipEntries" );
entryNames = "";
for ( row in zipEntries ) { entryNames = ListAppend( entryNames, row.name ); }
assertTrue( "the plain param is stored under its file name", ListFindNoCase( entryNames, "a.txt" ) > 0 );
assertTrue( "entrypath names the entry outright", ListFindNoCase( entryNames, "nested/b.txt" ) > 0 );
assert( "both params were collected", ListLen( entryNames ), 2 );

// --- cfinvoke { cfinvokeArgument } ------------------------------------------
function invokeWithChildArgs() {
	var greeter = createObject( "component", "tags.invoketarget.Greeter" );
	var results = [];

	cfinvoke( component=greeter, method="greet", returnvariable="local.r1" ) {
		cfinvokeArgument( name='who', value="world" );
	}
	ArrayAppend( results, r1 );

	// An argument given as a cfinvoke ATTRIBUTE merges with the child tags, and
	// a child inside `if` is still seen.
	var withPunct = true;
	cfinvoke( component=greeter, method="greet", returnvariable="r2", who="alex" ) {
		if ( withPunct ) { cfinvokeArgument( name="punct", value="?" ); }
	}
	ArrayAppend( results, r2 );

	return ArrayToList( results, "~" );
}
assert( "cfinvoke( … ) { cfinvokeArgument(…) } passes its arguments"
      , invokeWithChildArgs(), "hi world!~hi alex?" );

DirectoryDelete( zipDir, true );
suiteEnd();
</cfscript>
