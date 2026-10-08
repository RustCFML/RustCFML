<cfscript>
suiteBegin("Core: the binary operator words are legal identifiers");

// ============================================================
// Background
// ============================================================
// CFML's word operators (`eq`, `lt`, `le`, `gt`, `ge`, `mod`, `is`, `contains`,
// `and`, `or`, `xor`, `eqv`, `imp`) are operators only BETWEEN two operands.
// Everywhere a NAME is expected they are ordinary identifiers, and Lucee accepts
// them as such. RustCFML lexed them as operator tokens everywhere, so
// `catch ( any le ) {}` — in Preside's SharePoint API wrapper — was a parse
// error that took the whole application's boot with it.
//
// `le` and `ge` are ALIASES (they lex as `lte`/`gte`), so the source spelling
// has to survive: a variable the app called `le` must not come back as `lte`.
//
// `not` is deliberately NOT included: it is a PREFIX operator, so `not x` is a
// genuine expression and cannot double as a name in the same positions.
// ============================================================

// --- the shape that broke Preside's boot ---
caught = "";
try {
    throw( type="test.operatorWord", message="boom" );
} catch ( any le ) {
    caught = le.message;
}
assert( "a catch variable may be named 'le'", caught, "boom" );

// --- as ordinary variables, written and read ---
le = 1;
ge = 6;
eq = 2;
mod = 3;
contains = 4;
is = 5;
assert( "'le' holds its value", le, 1 );
assert( "'ge' holds its value", ge, 6 );
assert( "'eq' holds its value", eq, 2 );
assert( "'mod' holds its value", mod, 3 );
assert( "'contains' holds its value", contains, 4 );
assert( "'is' holds its value", is, 5 );
// The alias keeps the name the app wrote, rather than the canonical 'lte'.
// `ge` (not `le`) is used for the storage check: a name that has ALSO been a
// catch variable earlier in the file is shadowed by that binding for the rest
// of the page — true of any name, operator word or not, so it would not be
// testing the alias here.
assertTrue( "an alias spelling is stored under the name the app wrote",
    structKeyExists( variables, "ge" ) );
assertFalse( "...not under the canonical spelling", structKeyExists( variables, "gte" ) );

// --- as function parameters, and as named arguments at the call site ---
function opWords( required numeric le, numeric gt=0 ) {
    return arguments.le + arguments.gt;
}
assert( "operator words as parameters and named arguments", opWords( le=5, gt=2 ), 7 );

// --- as struct keys and member reads ---
s = { le=3, contains=4, mod=5 };
assert( "as a struct key", s.le, 3 );
assert( "as another struct key", s.contains, 4 );

// --- as a loop variable ---
sum = 0;
for ( le in [ 7, 8 ] ) { sum += le; }
assert( "as a for-in loop variable", sum, 15 );

// --- CONTROL: every one of them still works as an OPERATOR ---
assertTrue( "lte still compares", 1 lte 2 );
assertTrue( "le still compares", 1 le 2 );
assertTrue( "gte still compares", 2 gte 1 );
assertTrue( "ge still compares", 2 ge 1 );
assertTrue( "lt/gt still compare", 1 lt 2 && 2 gt 1 );
assertTrue( "eq/neq still compare", 1 eq 1 && 1 neq 2 );
assertTrue( "is still compares", 2 is 2 );
assert( "mod still divides", 5 mod 2, 1 );
assertTrue( "contains still searches", "abc" contains "b" );
assertTrue( "and/or/xor still combine", ( true and true ) && ( false or true ) && ( true xor false ) );
assertTrue( "not still negates", not false );

// ============================================================
// The wider sweep: reserved words in NAME SLOTS
// ============================================================
// A differential sweep of 75 keywords x 11 identifier positions against Lucee
// 7.1.0.204 found 103 shapes Lucee accepts and RustCFML rejected. The ones
// below are the positions whose surrounding syntax already fixes the token as a
// NAME — a catch variable, an argument name before its `=`, a for-in loop
// variable, a parameter name — plus reading such a variable back. No statement
// can begin in those slots, so accepting every keyword there is unambiguous.
// (What remains is listed in docs/known-issues.md: statement-position
// assignment for words like `if`/`while`, and `function`/`super`/`static`,
// where the keyword's own meaning really is in play.)

// a catch variable may be any reserved word, and reads back
caughtWords = "";
for ( w in [ "return", "case", "in", "do", "try" ] ) {
    // (each shape below is written out rather than evaluated, so a parse
    // failure in any one of them fails this file loudly)
}
try { throw( type="t", message="m1" ); } catch ( any return ) { caughtWords &= return.message; }
try { throw( type="t", message="m2" ); } catch ( any case )   { caughtWords &= case.message; }
try { throw( type="t", message="m3" ); } catch ( any in )     { caughtWords &= in.message; }
try { throw( type="t", message="m4" ); } catch ( any do )     { caughtWords &= do.message; }
assert( "reserved words name a catch variable and read back", caughtWords, "m1m2m3m4" );

// named arguments may be reserved words
function slots( case=0, in=0, return=0, required=0 ) {
    return arguments.case & arguments.in & arguments.return & arguments.required;
}
assert( "reserved words as parameter and argument names",
    slots( case=1, in=2, return=3, required=4 ), "1234" );

// a for-in loop variable may be a reserved word, and reads inside the body
forSum = "";
for ( case in [ "a", "b" ] ) { forSum &= case; }
assert( "a reserved word as a for-in loop variable", forSum, "ab" );

// `required` on its own is a parameter NAME, not the modifier
function bareRequired( required ) { return arguments.required; }
assert( "a bare 'required' is the parameter name", bareRequired( 9 ), 9 );

suiteEnd();
</cfscript>
