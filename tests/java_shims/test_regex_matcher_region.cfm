<cfscript>
suiteBegin("java.util.regex.Matcher: region(), reset(), regionStart/End and the no-match state (GH ##483)");

// Every expectation below was taken from Lucee 7.1 running the real
// java.util.regex, so this file checks both engines agree.
Pattern = createObject( "java", "java.util.regex.Pattern" );

// The reported case.
content = "<tagName>test</tagName>";
m = Pattern.compile( "<[[:space:]]*/[[:space:]]*tagName[[:space:]]*>", Pattern.CASE_INSENSITIVE ).matcher( content );
m.region( 0, len( content ) );
found = [];
while ( m.find() ) { arrayAppend( found, m.start() + 1 ); }
assert( "GH ##483: find() inside a region", arrayToList( found ), "14" );

// Matches stay inside the region and offsets stay relative to the whole input.
m = Pattern.compile( "\d" ).matcher( "1a2b3c4d5" );
m.region( 2, 7 );
found = [];
while ( m.find() ) { arrayAppend( found, m.group() & "@" & m.start() ); }
assert( "find() walks only the region", arrayToList( found ), "2@2,3@4,4@6" );

m = Pattern.compile( "cde" ).matcher( "abcdef" );
m.region( 0, 3 );
assertFalse( "a match may not run past the region end", m.find() );
m = Pattern.compile( "ab" ).matcher( "abcdef" );
m.region( 1, 6 );
assertFalse( "a match may not start before the region start", m.find() );

// Java's default bounds: anchoring (^/$ at the edges) and opaque (\b cannot see out).
m = Pattern.compile( "^abc$" ).matcher( "xxabcxx" );
m.region( 2, 5 );
assertTrue( "^ and $ match at the region edges", m.find() );
assert( "start() of an anchored region match", m.start(), 2 );
assert( "end() of an anchored region match", m.end(), 5 );
m = Pattern.compile( "\babc\b" ).matcher( "xabcx" );
m.region( 1, 4 );
assertTrue( "\b treats the region edge as a boundary", m.find() );

m = Pattern.compile( "(\d)(\w)" ).matcher( "1a2b3c" );
m.region( 2, 6 );
m.find();
assert( "group offsets inside a region", m.start( 1 ) & "," & m.end( 2 ) & "," & m.group( 2 ), "2,4,b" );

// matches()/lookingAt() apply to the region.
m = Pattern.compile( "abc" ).matcher( "xxabcxx" );
m.region( 2, 5 );
assertTrue( "matches() is the whole region", m.matches() );
m = Pattern.compile( "ab" ).matcher( "xxabcxx" );
m.region( 2, 5 );
assertTrue( "lookingAt() anchors at the region start", m.lookingAt() );
m = Pattern.compile( "ab" ).matcher( "xxabcxx" );
m.region( 2, 5 );
assertFalse( "matches() rejects part of the region", m.matches() );

// Character offsets, after multi-byte text.
m = Pattern.compile( "b" ).matcher( "ééabé b" );
m.region( 3, 7 );
found = [];
while ( m.find() ) { arrayAppend( found, m.start() ); }
assert( "char offsets in a region after multi-byte text", arrayToList( found ), "3,6" );

// An empty region yields one empty match.
m = Pattern.compile( "" ).matcher( "abcdef" );
m.region( 3, 3 );
found = [];
loops = 0;
while ( m.find() && loops++ < 10 ) { arrayAppend( found, m.start() ); }
assert( "an empty region matches once", arrayToList( found ), "3" );

// regionStart()/regionEnd(), region() rewinding, reset().
m = Pattern.compile( "a" ).matcher( "hello world" );
assert( "regionStart() defaults to 0", m.regionStart(), 0 );
assert( "regionEnd() defaults to the input length", m.regionEnd(), 11 );
m.region( 3, 8 );
assert( "regionStart() after region()", m.regionStart(), 3 );
assert( "regionEnd() after region()", m.regionEnd(), 8 );

m = Pattern.compile( "\d" ).matcher( "1a2b3" );
m.find();
m.find();
m.region( 0, 5 );
m.find();
assert( "region() rewinds the cursor", m.start(), 0 );

m = Pattern.compile( "\d" ).matcher( "1a2b3" );
m.region( 2, 5 );
m.find();
assert( "first find() in a region", m.start(), 2 );
m.reset();
assert( "reset() lifts the region (start)", m.regionStart(), 0 );
assert( "reset() lifts the region (end)", m.regionEnd(), 5 );
m.find();
assert( "reset() rewinds to the start", m.start(), 0 );
m.reset( "zz9" );
m.find();
assert( "reset(input) swaps the input", m.group() & "@" & m.start(), "9@2" );
assert( "regionEnd() follows the new input", m.regionEnd(), 3 );

// region() returns the matcher, so it chains.
assertTrue( "region() chains", Pattern.compile( "b" ).matcher( "abcb" ).region( 2, 4 ).find() );
x = Pattern.compile( "b" ).matcher( "abcb" ).region( 2, 4 );
x.find();
assert( "a chained region() keeps its bounds", x.start(), 3 );
m = Pattern.compile( "c" ).matcher( "abcdef" );
m.region( "1", "4" );
assertTrue( "numeric-string bounds are accepted", m.find() );

// Java's bounds checks, in Java's order.
try { Pattern.compile( "a" ).matcher( "abc" ).region( 2, 10 ); r = "nothrow"; } catch ( any e ) { r = e.type & "|" & e.message; }
assert( "end past the input", r, "java.lang.IndexOutOfBoundsException|end" );
try { Pattern.compile( "a" ).matcher( "abc" ).region( 2, 1 ); r = "nothrow"; } catch ( any e ) { r = e.type & "|" & e.message; }
assert( "start after end", r, "java.lang.IndexOutOfBoundsException|start > end" );
try { Pattern.compile( "a" ).matcher( "abc" ).region( -1, 1 ); r = "nothrow"; } catch ( any e ) { r = e.type & "|" & e.message; }
assert( "negative start", r, "java.lang.IndexOutOfBoundsException|start" );

// No current match: group()/start()/end() throw (they returned null/-1 here).
m = Pattern.compile( "(c)" ).matcher( "abc" );
try { m.group( 1 ); r = "nothrow"; } catch ( any e ) { r = e.type & "|" & e.message; }
assert( "group() before any find()", r, "java.lang.IllegalStateException|No match found" );
m.find();
m.find();
try { m.start(); r = "nothrow"; } catch ( any e ) { r = e.type & "|" & e.message; }
assert( "start() after a failed find()", r, "java.lang.IllegalStateException|No match found" );
m = Pattern.compile( "c" ).matcher( "abc" );
m.find();
m.region( 0, 3 );
try { m.group(); r = "nothrow"; } catch ( any e ) { r = e.type; }
assert( "region() discards the current match", r, "java.lang.IllegalStateException" );
m = Pattern.compile( "(x)?c" ).matcher( "abc" );
m.find();
assertTrue( "a group that did not take part is still null", isNull( m.group( 1 ) ) );
assert( "... and its start() is still -1", m.start( 1 ), -1 );

// Java has no POSIX bracket classes: [[:space:]] is the characters ":space".
assertFalse( "[[:space:]] does not match a blank", Pattern.compile( "a[[:space:]]b" ).matcher( "a b" ).matches() );
assertTrue( "[[:space:]] matches a colon", Pattern.compile( "a[[:space:]]b" ).matcher( "a:b" ).matches() );
assertTrue( "[[:alpha:]] is the letters of 'alpha'", Pattern.compile( "[[:alpha:]]+" ).matcher( "ahpl" ).matches() );
assertFalse( "String.matches() reads [[:space:]] the same way", javaCast( "string", "a b" ).matches( "a[[:space:]]b" ) );

suiteEnd();
</cfscript>
