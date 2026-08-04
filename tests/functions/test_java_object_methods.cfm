<!---
  java.lang.Object / Comparable methods on simple values —
  docs/known-issues.md §33.

  Lucee boxes a CFML simple value as a Java object, so equals/hashCode/compareTo
  are callable on one. RustCFML implemented them for String only; on every other
  receiver they threw ("The function [equals] does not exist in the Numeric."),
  which cost 8 tests in TestBox's own suite — `equalize` compares numerics itself
  and only reaches `actual.equals( expected )` once they have already differed,
  so the isNotEqual path threw where Lucee returns false.

  Every expected value below was read off Lucee 7.0.4. Two Lucee answers are
  deliberately NOT pinned here because they are artifacts of Lucee's numeric
  boxing rather than of these methods (it boxes `1` as a Long but `-1` and
  `4294967296` as Doubles, so their hashCodes differ from ours); see §33.
--->
<cfscript>
suiteBegin( "java.lang.Object methods on simple values (§33)" );

n = 1;
d = 1.5;
b = true;
s = "a";
arr = [ 1 ];
st = { a = 1 };

// --- equals: type-strict Java equality, no CFML coercion --------------------
assert( "int equals same int",        n.equals( 1 ),      true );
assert( "int equals other int",       n.equals( 2 ),      false );
assert( "int does not equal string",  n.equals( "1" ),    false );
assert( "int does not equal double",  n.equals( 1.0 ),    false );
assert( "int does not equal boolean", n.equals( true ),   false );
assert( "double equals same double",  d.equals( 1.5 ),    true );
assert( "double vs numeric string",   d.equals( "1.5" ),  false );
assert( "boolean equals boolean",     b.equals( true ),   true );
assert( "boolean vs string 'true'",   b.equals( "true" ), false );
assert( "boolean vs 1",               b.equals( 1 ),      false );
assert( "string equals same",         s.equals( "a" ),    true );
assert( "string equals is cased",     s.equals( "A" ),    false );

// A whole-number double still is not an int, on either engine.
wholeDouble = 2.0;
assert( "2.0 does not equal 2", wholeDouble.equals( 2 ), false );

// Array equals is java.util.List.equals — element-wise, order-sensitive.
assert( "array equals same",       arr.equals( [ 1 ] ),    true );
assert( "array differing element", arr.equals( [ 2 ] ),    false );
assert( "array differing length",  arr.equals( [ 1, 2 ] ), false );
assert( "nested arrays",           [ [ 1 ], 2 ].equals( [ [ 1 ], 2 ] ), true );

// Struct equals is Lucee's case-INSENSITIVE Struct.equals.
assert( "struct equals same",         st.equals( { a = 1 } ), true );
assert( "struct differing value",     st.equals( { a = 2 } ), false );
assert( "struct keys are insensitive", st.equals( { A = 1 } ), true );
assert( "nested structs", { a = { b = 1 } }.equals( { a = { b = 1 } } ), true );

// --- hashCode: the JVM's exact values --------------------------------------
assert( "Long.hashCode(1)",       n.hashCode(),        1 );
assert( "Double.hashCode(1.5)",   d.hashCode(),        1073217536 );
assert( "Double.hashCode(2.0)",   wholeDouble.hashCode(), 1073741824 );
assert( "Boolean.hashCode(true)", b.hashCode(),        1231 );
bFalse = false;
assert( "Boolean.hashCode(false)", bFalse.hashCode(), 1237 );
assert( "String.hashCode('a')",   s.hashCode(),        97 );
assert( "String.hashCode('ab')",  "ab".hashCode(),     3105 );

// java.util.List.hashCode: 31*h + element.
assert( "List.hashCode([1])",   arr.hashCode(),      32 );
assert( "List.hashCode([1,2])", [ 1, 2 ].hashCode(), 994 );
assert( "List.hashCode(['a'])", [ "a" ].hashCode(),  128 );
assert( "List.hashCode([])",    [].hashCode(),       1 );

// java.util.Map.hashCode: the SUM of per-entry keyHash^valueHash, with keys
// hashed in UPPER case (the casing Lucee's Struct stores them in) — so {a:1}
// is 64 ("A"=65 ^ 1), not 96 ("a"=97 ^ 1).
assert( "Map.hashCode({a:1})",     st.hashCode(),             64 );
assert( "Map.hashCode({A:1}) same", { A = 1 }.hashCode(),     64 );
assert( "Map.hashCode({b:1})",     { b = 1 }.hashCode(),      67 );
assert( "Map.hashCode({ab:1})",    { ab = 1 }.hashCode(),     2080 );
assert( "Map.hashCode({a:'a'})",   { a = "a" }.hashCode(),    32 );
assert( "Map.hashCode sums entries", { a = 1, b = 1 }.hashCode(), 131 );
assert( "Map.hashCode({})",        {}.hashCode(),             0 );

// --- compareTo: sign only ---------------------------------------------------
assert( "int compareTo greater", n.compareTo( 2 ), -1 );
assert( "int compareTo lesser",  n.compareTo( 0 ),  1 );
assert( "int compareTo equal",   n.compareTo( 1 ),  0 );
assert( "boolean false < true",  b.compareTo( false ), 1 );
assert( "string compareTo",      s.compareTo( "b" ), -1 );

// Array/Struct are not Comparable on either engine — both must still throw.
assertThrows( "array has no compareTo",  function() { return [ 1 ].compareTo( [ 1 ] ); } );
assertThrows( "struct has no compareTo", function() { return { a = 1 }.compareTo( { a = 1 } ); } );

// --- toString still resolves ------------------------------------------------
assert( "int toString",     n.toString(), "1" );
assert( "boolean toString", b.toString(), "true" );

suiteEnd();
</cfscript>
