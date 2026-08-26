<cfscript>
// The extension ships its own CFC. Nothing in this application declares a
// mapping for it — the engine mounts the extension's cfml/ as /demo/.
f = createObject( "component", "demo.Formatter" );
writeOutput( "demo.Formatter.slug()    = " & f.slug( "Hello, Extension World!" ) & chr(10) );
writeOutput( "demo.Formatter.slugAll() = " & arrayToList( f.slugAll( [ "One Two", "Three!" ] ) ) & chr(10) );
</cfscript>
