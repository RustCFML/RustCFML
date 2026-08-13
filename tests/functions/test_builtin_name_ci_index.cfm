<cfscript>
// The `LoadLocalKey`/`LoadVariablesKey` peephole asks "is this name a builtin?"
// to decide whether a same-named local/variables key may shadow the BIF. That
// probe used to be an exact-case HashMap hit ORed with a linear
// `eq_ignore_ascii_case` scan of all ~400 builtin keys; it is now a single
// lookup in `builtin_names_lc`, an ASCII-lowercased index of the same keys
// (~7% of CPU on a live Preside admin profile). Builtins are registered
// MIXED-case (`writeOutput`, `lCase`), so the index must answer identically for
// every casing — and the index carries a staleness check that falls back to the
// old path, which these tests also exercise via the normal registration route.
//
// Behaviour must be byte-identical to the pre-index engine. These lock that in.
suiteBegin("Builtin-name case-insensitive index");

// --- BIFs resolve under any casing ----------------------------------------
assert( "lcase all-lower", lcase( "AB" ), "ab" );
assert( "lCase registered casing", lCase( "AB" ), "ab" );
assert( "LCASE all-upper", LCASE( "AB" ), "ab" );
assert( "LcAsE mixed", LcAsE( "AB" ), "ab" );

assert( "ucase all-lower", ucase( "ab" ), "AB" );
assert( "UCase mixed", UCase( "ab" ), "AB" );

// A multi-word builtin whose registered form is camelCase.
assert( "arrayLen registered casing", arrayLen( [ 1, 2, 3 ] ), 3 );
assert( "arraylen all-lower", arraylen( [ 1, 2, 3 ] ), 3 );
assert( "ARRAYLEN all-upper", ARRAYLEN( [ 1, 2, 3 ] ), 3 );

// --- The lowerCamel registry vs UpperCamel-writing codebases ---------------
// Every one of these is registered lowerCamel, but Preside/ColdBox (and much
// CFML in the wild) spell them UpperCamel — which used to mean EVERY call fell
// into an unmemoized linear scan of all ~730 builtins (+236..635 ns/call, 2-4x
// the cost of the call itself). Resolution is now one hashed probe into a
// lowercased index, and these lock in that all four spellings agree.
assert( "len registry casing",   len( "abcd" ), 4 );
assert( "Len UpperCamel",        Len( "abcd" ), 4 );
assert( "LEN all-upper",         LEN( "abcd" ), 4 );
assert( "lEn mixed",             lEn( "abcd" ), 4 );

assert( "trim registry casing",  trim( "  x  " ), "x" );
assert( "Trim UpperCamel",       Trim( "  x  " ), "x" );
assert( "TRIM all-upper",        TRIM( "  x  " ), "x" );
assert( "tRiM mixed",            tRiM( "  x  " ), "x" );

assert( "listLast registry casing", listLast( "a,b,c" ), "c" );
assert( "ListLast UpperCamel",      ListLast( "a,b,c" ), "c" );
assert( "LISTLAST all-upper",       LISTLAST( "a,b,c" ), "c" );
assert( "lIsTlAsT mixed",           lIsTlAsT( "a,b,c" ), "c" );

kex = { a = 1 };
assertTrue( "structKeyExists registry casing", structKeyExists( kex, "a" ) );
assertTrue( "StructKeyExists UpperCamel",      StructKeyExists( kex, "a" ) );
assertTrue( "STRUCTKEYEXISTS all-upper",       STRUCTKEYEXISTS( kex, "a" ) );
assertTrue( "sTrUcTkEyExIsTs mixed",           sTrUcTkEyExIsTs( kex, "a" ) );
assertFalse( "StructKeyExists UpperCamel, absent key", StructKeyExists( kex, "zz" ) );

assert( "reReplace registry casing", reReplace( "a1b2", "[0-9]", "-", "all" ), "a-b-" );
assert( "ReReplace UpperCamel",      ReReplace( "a1b2", "[0-9]", "-", "all" ), "a-b-" );
assert( "REREPLACE all-upper",       REREPLACE( "a1b2", "[0-9]", "-", "all" ), "a-b-" );
assert( "rErEpLaCe mixed",           rErEpLaCe( "a1b2", "[0-9]", "-", "all" ), "a-b-" );

// A BIF that goes through the VM intercept list rather than the plain registry
// dispatch, so both halves of the resolution chain are covered.
dupSrc = { a = [ 1, 2 ] };
dupA = duplicate( dupSrc );
dupB = Duplicate( dupSrc );
dupC = DUPLICATE( dupSrc );
dupA.a[ 1 ] = 99;
dupB.a[ 1 ] = 98;
dupC.a[ 1 ] = 97;
assert( "duplicate is a deep copy (registry casing)", dupSrc.a[ 1 ], 1 );
assert( "Duplicate UpperCamel copies",  dupB.a[ 1 ], 98 );
assert( "DUPLICATE all-upper copies",   dupC.a[ 1 ], 97 );

// Nested/argument position: the resolution runs per call site, so a miscased
// BIF inside another miscased BIF's arguments must resolve too.
assert( "nested miscased BIFs", Len( Trim( ListLast( " a,b,cde " ) ) ), 3 );

// --- User functions resolve under any casing, and still shadow ------------
function ciHelperFn( required string s ) {
	return "helper:" & arguments.s;
}
assert( "UDF declared casing",    ciHelperFn( "x" ),  "helper:x" );
assert( "UDF all-lower",          cihelperfn( "x" ),  "helper:x" );
assert( "UDF all-upper",          CIHELPERFN( "x" ),  "helper:x" );
assert( "UDF mixed casing",       CiHeLpErFn( "x" ),  "helper:x" );

// A UDF declared UpperCamel is reachable lowerCamel — the mirror image.
function CIUpperHelper() {
	return "upper";
}
assert( "UpperCamel UDF, declared casing", CIUpperHelper(), "upper" );
assert( "UpperCamel UDF, lower call",      ciupperhelper(), "upper" );

// Resolution ORDER is unchanged by the index: a scope entry holding a function
// still wins over the same-named entry in the user-function table, whatever
// casing either is spelled with (scopes are searched before user functions,
// which are searched before builtins).
function ciOrderProbe() {
	return "declared";
}
assert( "UDF before override", ciOrderProbe(), "declared" );
CIORDERPROBE = function() {
	return "override";
};
assert( "scope function wins over the UDF table", ciOrderProbe(), "override" );
assert( "...under any casing",                    CiOrDeRpRoBe(), "override" );

// --- A local named after a builtin still CALLS the builtin -----------------
// This is the exact case the peephole comment documents: the data hit is
// skipped in call position so the BIF stays reachable.
function localNamedLikeBuiltin() {
	var lcase = { a = 1 };
	// call position -> the BIF, not the struct
	return lcase( "XY" );
}
assert( "local named `lcase` still calls the BIF", localNamedLikeBuiltin(), "xy" );

// ...and in READ position the local wins (a data hit is always visible there).
function readLocalNamedLikeBuiltin() {
	var lcase = "i am data";
	return lcase;
}
assert( "local named `lcase` reads as data", readLocalNamedLikeBuiltin(), "i am data" );

// NB: a local declared in a DIFFERENT casing than the builtin (`var LCase = …`
// then a bare `lcase` read) currently resolves to the BIF, not the local —
// a case-insensitivity divergence that PREDATES the index (the builtin probe is
// case-insensitive both before and after it; the defect is in whether the local
// hit is seen). Deliberately not asserted here so this file doesn't encode
// known-wrong behaviour; tracked separately.

// --- variables-scope equivalent -------------------------------------------
variables.ucase = "page data";
assert( "variables.ucase reads as data", variables.ucase, "page data" );
assert( "ucase() still calls the BIF", ucase( "zz" ), "ZZ" );

// --- a name that is NOT a builtin must not be treated as one ---------------
function notABuiltin() {
	var definitelynotabuiltinname = "plain";
	return definitelynotabuiltinname;
}
assert( "non-builtin local reads normally", notABuiltin(), "plain" );

suiteEnd();
</cfscript>
