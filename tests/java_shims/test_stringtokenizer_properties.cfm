<cfscript>
suiteBegin("java shim: java.util.StringTokenizer and java.util.Properties");

// ---- StringTokenizer -------------------------------------------------------
// Preside's EmailStyleInliner walks CSS with the countTokens()>1 idiom, which
// depends on countTokens() meaning REMAINING and nextToken() advancing in place.
css = "a { color: red; } p { margin: 0; }";
tok = CreateObject( "java", "java.util.StringTokenizer" ).init( css, "{}" );

assert( "countTokens reports the total before consuming", tok.countTokens(), 4 );
pairs = [];
while ( tok.countTokens() > 1 ) {
	arrayAppend( pairs, trim( tok.nextToken() ) & "=>" & trim( tok.nextToken() ) );
}
assert( "the selector/style pairs come out in order", arrayToList( pairs, "|" ), "a=>color: red;|p=>margin: 0;" );
assert( "countTokens counts what REMAINS", tok.countTokens(), 0 );
assertFalse( "hasMoreTokens is false once drained", tok.hasMoreTokens() );
assertThrows( "nextToken past the end throws", function(){ tok.nextToken(); } );

// Java's default delimiter set is whitespace, and runs of delimiters collapse.
ws = CreateObject( "java", "java.util.StringTokenizer" ).init( "  one   two" & chr(9) & "three " );
assert( "whitespace is the default delimiter set, runs collapsed", ws.countTokens(), 3 );
assert( "first whitespace-delimited token", ws.nextToken(), "one" );

// returnDelims yields each delimiter as its own token.
rd = CreateObject( "java", "java.util.StringTokenizer" ).init( "a,b", ",", true );
assert( "returnDelims emits the delimiters too", rd.countTokens(), 3 );
rd.nextToken();
assert( "the delimiter is its own token", rd.nextToken(), "," );

// ---- Properties ------------------------------------------------------------
// Built to configure something else — a JavaMail Session, a driver, a bundle.
props = CreateObject( "java", "java.util.Properties" ).init();
props.put( "mail.smtp.starttls.enable", "true" );
props.put( "mail.smtp.auth", "true" );

assert( "getProperty reads a value back", props.getProperty( "mail.smtp.auth" ), "true" );
assert( "get is the Map spelling of the same thing", props.get( "mail.smtp.auth" ), "true" );
assert( "size counts only real entries, not shim bookkeeping", props.size(), 2 );
assertTrue( "containsKey finds a set property", props.containsKey( "mail.smtp.auth" ) );
assertFalse( "containsKey is false for an absent one", props.containsKey( "mail.smtp.nope" ) );
assert( "getProperty falls back to the supplied default", props.getProperty( "mail.smtp.nope", "fallback" ), "fallback" );
assertNull( "getProperty with no default and no value is null", props.getProperty( "mail.smtp.nope" ) );

// setProperty is put's alias, and both return the displaced value.
assert( "setProperty returns the previous value", props.setProperty( "mail.smtp.auth", "false" ), "true" );
assert( "and the new value sticks", props.getProperty( "mail.smtp.auth" ), "false" );

names = props.stringPropertyNames();
assert( "stringPropertyNames lists the keys", arrayLen( names ), 2 );
assertTrue( "and excludes the shim's internal keys"
          , !arrayFindNoCase( names, "__java_class" ) && !arrayFindNoCase( names, "__java_shim" ) );

assert( "remove returns what it removed", props.remove( "mail.smtp.auth" ), "false" );
assert( "and the entry is gone", props.size(), 1 );

props.clear();
assertTrue( "clear empties the map", props.isEmpty() );

// Values are stringified, as they are in Java.
props.put( "port", 587 );
assert( "a numeric value is stored as its string form", props.getProperty( "port" ), "587" );

// load()/store() would need a stream and a full .properties parser — refused
// rather than half-implemented.
loadType = "";
try { props.load( "whatever" ); } catch ( any e ) { loadType = e.type; }
assert( "load() is refused, not faked", loadType, "java.lang.UnsupportedOperationException" );

suiteEnd();
</cfscript>
