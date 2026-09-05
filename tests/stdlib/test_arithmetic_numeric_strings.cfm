<cfscript>
suiteBegin("Arithmetic + on numeric strings");

// CFML `+` is ARITHMETIC ONLY (`&` is concatenation). Two numeric strings must
// ADD, not concatenate. RustCFML previously had a String+String fast path in
// the Add op that concatenated "2"+"1" → "21", inverting
// Wheels' if/unless method-mixin validations (`stupid_mixin(a,b){return a+b}`).
assert("two numeric strings add", "2" + "1", 3);
assert("multi-digit numeric strings add", "20" + "13", 33);
assert("numeric string + int", "2" + 5, 7);
assert("int + numeric string", 5 + "2", 7);
assert("decimal strings add", "1.5" + "2.5", 4);

// `&` still concatenates (unchanged).
assert("ampersand still concatenates", "2" & "1", "21");

// A function mixin returning a+b on string args (the Wheels pattern).
mixin = function(a, b) { return a + b; };
assert("mixin a+b on string args", mixin("2", "1"), 3);

// A non-numeric operand THROWS — CFML arithmetic is numeric-only and `&` is
// the concatenation operator (GH #350). This suite used to assert the opposite
// ("non-numeric + concatenates"), which is why the divergence went unreported:
// the wrong answer was written in as the expected one. Messages measured on
// Lucee 7.1.0.204; note it words the empty string differently.
function threwWith( body ) {
    try { body(); return "(did not throw)"; } catch ( any e ) { return e.message; }
}
assert( "non-numeric + throws",  threwWith( function(){ return "foo" + "bar"; } ),
        "can't cast [foo] string to a number value" );
assert( "non-numeric * throws",  threwWith( function(){ return "foo" * "bar"; } ),
        "can't cast [foo] string to a number value" );
// `/` names the DIVISOR, not the left operand — Lucee coerces it first.
assert( "non-numeric / throws",  threwWith( function(){ return "foo" / "bar"; } ),
        "can't cast [bar] string to a number value" );
assert( "non-numeric - throws",  threwWith( function(){ return "foo" - "bar"; } ),
        "can't cast [foo] string to a number value" );
assert( "number + non-numeric throws", threwWith( function(){ return 2 + "abc"; } ),
        "can't cast [abc] string to a number value" );
assert( "empty string is its own message", threwWith( function(){ return "" + 1; } ),
        "can't cast empty string to a number value" );
assert( "a complex operand throws", threwWith( function(){ return {} + 1; } ),
        "can't cast Complex Object Type [Struct] to a number value" );

// ...but everything Lucee DOES coerce still coerces.
assert( "boolean coerces",            true + 1, 2 );
assert( "boolean-literal string coerces", "yes" + 1, 2 );
assert( "surrounding whitespace is fine", " 2 " + 1, 3 );
assert( "a date coerces to its serial", createDate( 2020, 1, 1 ) + 1, 43832 );

suiteEnd();
</cfscript>
