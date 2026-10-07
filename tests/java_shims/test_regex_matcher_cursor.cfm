<cfscript>
suiteBegin("java.util.regex.Matcher: find() walks matches with Java's offsets and empty-match rule");

// RustCFML's Matcher shim used to rescan from the start for every find() and copy
// the input each time. It now resumes from a cursor; these pin the Java semantics
// (Lucee runs the real java.util.regex, so this file checks both engines agree).
Pattern = createObject( "java", "java.util.regex.Pattern" );

m = Pattern.compile( "<!--ds:\(type=(js|css),group=([a-z]+)\)\((.+?)\):ds-->" ).matcher( javaCast( "string",
	"head <!--ds:(type=js,group=top)(a.js):ds--> mid <!--ds:(type=css,group=base)(b.css):ds--> tail" ) );
found = [];
while ( m.find() ) {
	arrayAppend( found, m.group( 1 ) & "/" & m.group( 2 ) & "/" & m.group( 3 ) & "@" & m.start() & "-" & m.end() );
}
assert( "every match, in order, with groups and offsets", arrayToList( found, "|" ), "js/top/a.js@5-43|css/base/b.css@48-89" );
assertFalse( "find() after the last match stays false", m.find() );

// Offsets are CHARACTER offsets, also after multi-byte text.
m = Pattern.compile( "b+" ).matcher( javaCast( "string", "ééb ébb" ) );
offs = [];
while ( m.find() ) { arrayAppend( offs, m.start() & "-" & m.end() & ":" & m.group() ); }
assert( "char offsets after multi-byte characters", arrayToList( offs, "|" ), "2-3:b|5-7:bb" );

// Empty matches advance one character (Java: a* on "baa" -> "", "aa", "").
m = Pattern.compile( "a*" ).matcher( javaCast( "string", "baa" ) );
seen = [];
loops = 0;
while ( m.find() && loops++ < 10 ) { arrayAppend( seen, m.start() & "-" & m.end() ); }
assert( "empty matches advance and the loop terminates", arrayToList( seen, "|" ), "0-0|1-3|3-3" );

// Group start/end per group.
m = Pattern.compile( "(\d+)-(\d+)" ).matcher( javaCast( "string", "x 12-345 y" ) );
assertTrue( "matches", m.find() );
assert( "group 2 start", m.start( 2 ), 5 );
assert( "group 2 end", m.end( 2 ), 8 );

// javaCast("string", s) hands back an equal string.
s = repeatString( "abc", 1000 );
assert( "javaCast string round-trips", javaCast( "string", s ), s );

suiteEnd();
</cfscript>
