<cfscript>
suiteBegin("compare / compareNoCase / string == and < > (allocation-free case folding)");

// compareNoCase used to lowercase BOTH operands into new strings per call, and
// string `==` lowercased both just to test for true/yes/false/no. The answers
// must not change now that neither allocates.
assert( "compareNoCase equal, different case", compareNoCase( "getName", "GETNAME" ), 0 );
assert( "compareNoCase less", compareNoCase( "abc", "ABD" ), -1 );
assert( "compareNoCase greater", compareNoCase( "ABD", "abc" ), 1 );
assert( "compareNoCase prefix is less", compareNoCase( "abc", "ABCD" ), -1 );
assert( "compareNoCase longer is greater", compareNoCase( "ABCD", "abc" ), 1 );
assert( "compareNoCase empty strings", compareNoCase( "", "" ), 0 );
assert( "compareNoCase empty is less", compareNoCase( "", "a" ), -1 );
assert( "compareNoCase underscore vs letter folds the letter", compareNoCase( "_x", "X" ), -1 );
assert( "compareNoCase non-ASCII folds", compareNoCase( "ÄBC", "äbc" ), 0 );
assert( "compareNoCase number args compare as strings", compareNoCase( 10, 9 ), -1 );

assert( "compare is case-sensitive (upper sorts first)", compare( "A", "a" ), -1 );
assert( "compare equal", compare( "init", "init" ), 0 );
assert( "compare greater", compare( "b", "a" ), 1 );
assert( "compare number args compare as strings", compare( 10, 9 ), -1 );

assertTrue( "== ignores case", "abc" == "ABC" );
assertFalse( "== different strings", "abc" == "abd" );
assertTrue( "== yes equals true", "yes" == "true" );
assertTrue( "== YES equals TRUE", "YES" == "TRUE" );
assertTrue( "== 1 equals true", "1" == "true" );
assertTrue( "== no equals false", "no" == "false" );
assertFalse( "== yes is not no", "yes" == "no" );
assertFalse( "== yesterday is not a boolean literal", "yesterday" == "true" );

assertTrue( "lt is case-insensitive", "apple" lt "Banana" );
assertTrue( "gt is case-insensitive", "b" gt "A" );
assertFalse( "neither lt nor gt when equal ignoring case", ( "Init" lt "init" ) or ( "Init" gt "init" ) );

suiteEnd();
</cfscript>
