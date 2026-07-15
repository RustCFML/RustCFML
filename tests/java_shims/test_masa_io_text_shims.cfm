<cfscript>
// Java shims surfaced booting Masa CMS admin:
//   - java.text.MessageFormat  (resourceBundle locale-aware message formatting)
//   - java.io.BufferedReader   (resourceBundle .properties line reader)
//   - java.util.HashMap        (admin index)
suiteBegin( "Java Shims: MessageFormat / BufferedReader / HashMap (Masa)" );

// ---- java.text.MessageFormat ----
mf = createObject( "java", "java.text.MessageFormat" );
loc = createObject( "java", "java.util.Locale" );

f1 = mf.init( "Hello {0}, you have {1,number} msgs", loc );
assert( "MessageFormat positional + grouped number",
    f1.format( [ "Bob", 1234 ] ), "Hello Bob, you have 1,234 msgs" );

f2 = mf.init( "plain {0} and {1}" );
assert( "MessageFormat plain positional", f2.format( [ "a", "b" ] ), "plain a and b" );

f3 = mf.init( "Total: {0,number,integer}" );
assert( "MessageFormat integer style rounds+groups", f3.format( [ 9999.7 ] ), "Total: 10,000" );

// MessageFormat quoting: '' -> literal apostrophe, 'text' -> literal, {x} with a
// non-numeric arg index passes through verbatim.
f4 = mf.init( "it''s {not a param}" );
assert( "MessageFormat quoting + non-index passthrough", f4.format( [] ), "it's {not a param}" );

// ---- java.io.BufferedReader over a FileInputStream/InputStreamReader chain ----
tmpFile = getTempDirectory() & "rcfml_masa_br_" & createUUID() & ".properties";
commentLine = chr(35) & "comment";
fileWrite( tmpFile, "foo=bar" & chr(10) & commentLine & chr(10) & "baz=qux qux" & chr(10) & "empty=" );

fis  = createObject( "java", "java.io.FileInputStream" ).init( tmpFile );
fisr = createObject( "java", "java.io.InputStreamReader" ).init( fis, "UTF-8" );
br   = createObject( "java", "java.io.BufferedReader" ).init( fisr );

readLines = [];
do {
    line = br.readLine();
    haveLine = isDefined( "line" );
    if ( haveLine ) {
        arrayAppend( readLines, line );
    }
} while ( haveLine );
br.close();
fileDelete( tmpFile );

assert( "BufferedReader read all lines", arrayLen( readLines ), 4 );
assert( "BufferedReader line 1", readLines[ 1 ], "foo=bar" );
assert( "BufferedReader keeps comment line verbatim", readLines[ 2 ], commentLine );
assert( "BufferedReader line 3 (value with space)", readLines[ 3 ], "baz=qux qux" );
assert( "BufferedReader trailing empty value line", readLines[ 4 ], "empty=" );

// ---- java.util.HashMap (aliased to the ordered map shim) ----
hm = createObject( "java", "java.util.HashMap" ).init();
hm.put( "one", 1 );
hm.put( "two", 2 );
assertTrue( "HashMap size", hm.size() == 2 );
assertTrue( "HashMap get", hm.get( "two" ) == 2 );
assertTrue( "HashMap containsKey", hm.containsKey( "one" ) );

suiteEnd();
</cfscript>
