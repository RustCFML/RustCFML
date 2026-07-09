<cfscript>
suiteBegin( "struct case-insensitive index (issue ##262) — O(1) ci ops, casing preserved" );

// Issue #262: case-insensitive struct ops (StructKeyExists, ci reads, inserts)
// used to fall back to an O(n) linear scan on miss / case-mismatch, degrading
// large-struct build+probe to O(n^2)-O(n^4). The fix keys a secondary
// fold->original-key index so every ci op is O(1). These assertions lock in the
// *behavioural* invariants that fix must preserve; the perf win is verified out
// of band. Cross-engine safe (runs on Lucee too).

// --- first-written casing wins; later casings overwrite value in place ---
s = {};
s[ "Foo" ] = 1;
s[ "FOO" ] = 2;               // ci hit under different casing -> update in place
assert( "ci dedup keeps single entry", structCount( s ), 1 );
assert( "first casing preserved in key list", structKeyList( s ), "Foo" );
assert( "value overwritten by later casing", s[ "foo" ], 2 );

// --- ci reads resolve any casing ---
assert( "read exact case", s.Foo, 2 );
assert( "read lower case", s[ "foo" ], 2 );
assert( "read upper case", s[ "FOO" ], 2 );

// --- StructKeyExists is case-insensitive, hit and miss ---
assertTrue( "exists exact", structKeyExists( s, "Foo" ) );
assertTrue( "exists other casing", structKeyExists( s, "fOo" ) );
assertFalse( "exists miss", structKeyExists( s, "bar" ) );

// --- case-insensitive delete removes the single entry ---
structDelete( s, "FOO" );
assert( "ci delete removes entry", structCount( s ), 0 );
assertFalse( "gone after delete", structKeyExists( s, "foo" ) );

// --- build then probe a larger struct: exact + ci-mismatch + miss all correct ---
big = {};
for ( i = 1; i <= 500; i++ ) { big[ "Key_" & i ] = i; }
assert( "build count", structCount( big ), 500 );
assert( "exact-case read", big[ "Key_250" ], 250 );
assert( "ci-mismatch read", big[ "key_250" ], 250 );          // lower probe
assert( "ci-mismatch read upper", big[ "KEY_250" ], 250 );    // upper probe
assertTrue( "exists ci-mismatch", structKeyExists( big, "kEy_500" ) );
assertFalse( "exists miss on large struct", structKeyExists( big, "key_9999" ) );

// re-inserting under a different casing must not fork a second entry
big[ "KEY_1" ] = 111;
assert( "no fork on ci re-insert", structCount( big ), 500 );
assert( "ci re-insert updated value", big[ "key_1" ], 111 );

// --- structAppend (goes through the merge path) keeps ci semantics ---
// NB: assert on count/value, NOT structKeyList ORDER — RustCFML structs are
// insertion-ordered (IndexMap) but Lucee's default `{}` struct is unordered,
// so key-list order is not a cross-engine contract. Casing preservation IS
// (covered by the "first casing preserved in key list" 1-key check above).
a = { Alpha = 1 };
structAppend( a, { ALPHA = 9, Beta = 2 } );
assert( "append ci-merge count", structCount( a ), 2 );
assert( "append overwrote ci key", a[ "alpha" ], 9 );

// --- structClear empties both the map and the ci index ---
structClear( a );
assert( "clear empties", structCount( a ), 0 );
a[ "Gamma" ] = 5;             // rebuild after clear still resolves ci
assert( "insert after clear", a[ "gamma" ], 5 );

suiteEnd();
</cfscript>
