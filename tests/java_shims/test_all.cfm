<cfscript>
suiteBegin( "Java Shims" );

// These tests exercise real-world Lucee Java-interop patterns. The goal is
// that the same CFML code runs identically on Lucee (real JVM) and RustCFML
// (shimmed via createObject("java", …)). Avoid patterns that only work in
// one engine — e.g. calling .init() on a class that has no public
// constructor, or initializing TreeMap/LinkedHashMap from a CFML struct
// literal (Lucee uppercases unquoted struct keys, so later .get("a") fails).

// Test StringBuilder
writeOutput( "\n--- Testing StringBuilder ---\n" );
sb = createObject( "java", "java.lang.StringBuilder" ).init( "" );
// isObject() returns false on Lucee for StringBuilder (auto-coerces to String);
// isInstanceOf is the portable check.
assertTrue( "StringBuilder init", isInstanceOf( sb, "java.lang.StringBuilder" ) );

sb.append( "Hello" ).append( " " ).append( "World" );
result = sb.toString();
test1 = result == "Hello World";
assertTrue( "StringBuilder append", test1 );
test2 = sb.length() == 11;
assertTrue( "StringBuilder length", test2 );

// Test System — static class, no .init() (real Java has no public ctor)
writeOutput( "\n--- Testing System ---\n" );
system = createObject( "java", "java.lang.System" );
assertTrue( "System available", isObject( system ) );

osName = system.getProperty( "os.name" );
test3 = len( osName ) > 0;
assertTrue( "System.getProperty os.name", test3 );

fileSep = system.getProperty( "file.separator" );
test4 = len( fileSep ) == 1;
assertTrue( "System.getProperty file.separator", test4 );

userDir = system.getProperty( "user.dir" );
test5 = len( userDir ) > 0;
assertTrue( "System.getProperty user.dir", test5 );

// Test System.out
out = system.out;
assertTrue( "System.out exists", isObject( out ) );

// Test Date — real-world Lucee code commonly uses java.util.Date(0) as an
// epoch base date for dateDiff/dateAdd calculations.
writeOutput( "\n--- Testing Date ---\n" );
dateError = "";
dateMillis = "";
try {
    epochDate = createObject( "java", "java.util.Date" ).init( javacast( "int", 0 ) );
    dateMillis = epochDate.getTime();
} catch ( any e ) {
    dateError = e.message;
}
assert( "Date init does not throw", dateError, "" );
assert( "Date getTime epoch millis", dateMillis, 0 );

// Test File
writeOutput( "\n--- Testing File ---\n" );
file = createObject( "java", "java.io.File" ).init( "/tmp/test.txt" );
assertTrue( "File init", isObject( file ) );

assertTrue( "File isAbsolute", file.isAbsolute() );
test6 = len( file.getCanonicalPath() ) > 0;
assertTrue( "File getCanonicalPath", test6 );
test7 = file.toString() == "/tmp/test.txt";
assertTrue( "File toString", test7 );

// Test Thread — thread name varies by runtime; assert it's present, not "main"
writeOutput( "\n--- Testing Thread ---\n" );
thread = createObject( "java", "java.lang.Thread" ).currentThread();
assertTrue( "Thread currentThread", isObject( thread ) );

threadName = thread.getName();
assertTrue( "Thread getName returns string", len( threadName ) > 0 );

threadGroup = thread.getThreadGroup();
assertTrue( "Thread getThreadGroup", isObject( threadGroup ) );
test9 = len( threadGroup.getName() ) > 0;
assertTrue( "ThreadGroup getName", test9 );

// Test TreeMap — populate via put() so we control exact key casing.
writeOutput( "\n--- Testing TreeMap ---\n" );
map = createObject( "java", "java.util.TreeMap" ).init();
map.put( "z", 3 );
map.put( "a", 1 );
map.put( "m", 2 );
assertTrue( "TreeMap init", isInstanceOf( map, "java.util.TreeMap" ) );

keys = map.keySet().toArray();   // .toArray() needed: Lucee keySet() returns java.util.Set; toArray() gives Object[] which indexes cleanly in CFML
test10 = arrayLen( keys ) == 3;
assertTrue( "TreeMap keySet count", test10 );
test11 = keys[ 1 ] == "a";
assertTrue( "TreeMap sorted keys[1]", test11 );
test12 = keys[ 2 ] == "m";
assertTrue( "TreeMap sorted keys[2]", test12 );
test13 = keys[ 3 ] == "z";
assertTrue( "TreeMap sorted keys[3]", test13 );

test14 = map.get( "a" ) == 1;
assertTrue( "TreeMap get", test14 );
test15 = map.size() == 3;
assertTrue( "TreeMap size", test15 );
assertTrue( "TreeMap containsKey", map.containsKey( "a" ) );

// Test LinkedHashMap — likewise populate via put().
writeOutput( "\n--- Testing LinkedHashMap ---\n" );
lmap = createObject( "java", "java.util.LinkedHashMap" ).init();
lmap.put( "first", 1 );
lmap.put( "second", 2 );
lmap.put( "third", 3 );
assertTrue( "LinkedHashMap init", isInstanceOf( lmap, "java.util.LinkedHashMap" ) );

lkeys = lmap.keySet().toArray();
test16 = arrayLen( lkeys ) == 3;
assertTrue( "LinkedHashMap keySet count", test16 );
test17 = lkeys[ 1 ] == "first";
assertTrue( "LinkedHashMap order[1]", test17 );
test18 = lkeys[ 2 ] == "second";
assertTrue( "LinkedHashMap order[2]", test18 );
test19 = lkeys[ 3 ] == "third";
assertTrue( "LinkedHashMap order[3]", test19 );

test20 = lmap.get( "first" ) == 1;
assertTrue( "LinkedHashMap get", test20 );
test21 = lmap.size() == 3;
assertTrue( "LinkedHashMap size", test21 );

// Test ConcurrentLinkedQueue
writeOutput( "\n--- Testing ConcurrentLinkedQueue ---\n" );
queue = createObject( "java", "java.util.concurrent.ConcurrentLinkedQueue" ).init();
assertTrue( "Queue init", isObject( queue ) );

queue.offer( "a" );
queue.offer( "b" );
queue.offer( "c" );

test22 = queue.size() == 3;
assertTrue( "Queue size", test22 );
test23 = queue.peek() == "a";
assertTrue( "Queue peek", test23 );
test24 = queue.poll() == "a";
assertTrue( "Queue poll", test24 );
test25 = queue.size() == 2;
assertTrue( "Queue size after poll", test25 );

// Test Paths — obtained via File.toPath() rather than Paths.get(), because
// java.nio.file.Paths.get(String, String...) has varargs that Lucee can't
// resolve cleanly from CFML. File.toPath() is the portable route.
writeOutput( "\n--- Testing Paths ---\n" );
paths = createObject( "java", "java.io.File" ).init( "/home/user/file.txt" ).toPath();
assertTrue( "Paths get", isObject( paths ) );

parent = paths.getParent();
assertTrue( "Path getParent", isObject( parent ) );
test26 = parent.toString() == "/home/user";
assertTrue( "Parent toString", test26 );

assertTrue( "Path isAbsolute", paths.isAbsolute() );
test27 = paths.toString() == "/home/user/file.txt";
assertTrue( "Path toString", test27 );

// Test java.util.regex.Pattern — used by Lucee apps for dynamic route matching.
writeOutput( "\n--- Testing Pattern ---\n" );
pattern = createObject( "java", "java.util.regex.Pattern" );
matcher = pattern
    .compile( javaCast( "string", "^\/sysadmin\/routes\/([0-9A-Za-z\s\-_]+)$" ) )
    .matcher( javaCast( "string", "/sysadmin/routes/4e4727bd-f93d-40e6-9409-bed317fe76df" ) );

assertTrue( "Pattern matcher find", matcher.find() );
assert( "Pattern matcher group", matcher.group( 1 ), "4e4727bd-f93d-40e6-9409-bed317fe76df" );
assert( "Pattern matcher groupCount", matcher.groupCount(), 1 );

// find() must advance like Java's Matcher: a while(find()) loop walks every
// non-overlapping match in order, then terminates once they are exhausted.
multiMatcher = createObject( "java", "java.util.regex.Pattern" )
    .compile( javaCast( "string", "([0-9]+)" ) )
    .matcher( javaCast( "string", "a1b22c333" ) );
foundNumbers = [];
while ( multiMatcher.find() ) {
    arrayAppend( foundNumbers, multiMatcher.group( 1 ) );
}
assert( "Pattern find loop walks all matches", arrayToList( foundNumbers ), "1,22,333" );
assertFalse( "Pattern find returns false once exhausted", multiMatcher.find() );

// Matcher.start()/end() — 0-based char offsets of the most recent match,
// needed for in-place find-and-replace (Preside DynamicFindAndReplaceService).
// Without these the service prepended all replacements and left tokens intact.
offsetMatcher = createObject( "java", "java.util.regex.Pattern" )
    .compile( javaCast( "string", "\{\{.*?\}\}" ) )
    .matcher( javaCast( "string", "Lorem {{a}} ipsum {{bb}} sit." ) );
assertTrue( "start/end matcher finds first", offsetMatcher.find() );
assert( "Matcher.start() of first match", offsetMatcher.start(), 6 );
assert( "Matcher.end() of first match", offsetMatcher.end(), 11 );
assertTrue( "start/end matcher finds second", offsetMatcher.find() );
assert( "Matcher.start() of second match", offsetMatcher.start(), 18 );
assert( "Matcher.end() of second match", offsetMatcher.end(), 24 );

// Full in-place find-and-replace using start()/end() to slice literal gaps.
far_source  = "Lorem {{a}} ipsum {{bb}} sit.";
far_matcher = createObject( "java", "java.util.regex.Pattern" )
    .compile( javaCast( "string", "\{\{.*?\}\}" ) )
    .matcher( javaCast( "string", far_source ) );
far_builder = [];
far_pos = 1;
while ( far_matcher.find() ) {
    far_start = far_matcher.start() + 1;
    if ( far_pos < far_start ) {
        arrayAppend( far_builder, mid( far_source, far_pos, far_start - far_pos ) );
    }
    arrayAppend( far_builder, "[" & far_matcher.group() & "]" );
    far_pos = far_matcher.end() + 1;
}
if ( len( far_source ) >= far_pos ) {
    arrayAppend( far_builder, right( far_source, len( far_source ) - ( far_pos - 1 ) ) );
}
assert( "in-place find-and-replace preserves order", arrayToList( far_builder, "" ), "Lorem [{{a}}] ipsum [{{bb}}] sit." );

// MessageDigest — digest() must produce the REAL hash of the fed bytes, not
// echo them back. Known SHA-256 of "abc" and cross-check against hash().
md = createObject( "java", "java.security.MessageDigest" ).getInstance( "SHA-256" );
md.update( "abc".getBytes() );
mdHex = lCase( binaryEncode( md.digest(), "hex" ) );
assert(
    "MessageDigest SHA-256 of 'abc' matches known digest",
    mdHex,
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
);
assert( "MessageDigest SHA-256 agrees with hash()", mdHex, lCase( hash( "abc", "SHA-256" ) ) );

// update() accumulates across calls, exactly like Java.
md2 = createObject( "java", "java.security.MessageDigest" ).getInstance( "SHA-256" );
md2.update( "ab".getBytes() );
md2.update( "c".getBytes() );
assert( "MessageDigest update() accumulates chunks", lCase( binaryEncode( md2.digest(), "hex" ) ), mdHex );

// MD5 (the default algorithm) also hashes for real.
md5 = createObject( "java", "java.security.MessageDigest" ).getInstance( "MD5" );
md5.update( "abc".getBytes() );
assert( "MessageDigest MD5 of 'abc'", lCase( binaryEncode( md5.digest(), "hex" ) ), "900150983cd24fb0d6963f7d28e17f72" );

writeOutput( "\n=== All Java shim tests passed! ===\n" );

suiteEnd( "Java Shims" );
</cfscript>
