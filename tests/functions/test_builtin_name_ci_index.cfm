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
