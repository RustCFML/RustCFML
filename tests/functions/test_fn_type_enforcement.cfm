<cfsetting enablecfoutputonly="true">
<cfscript>
// Declared parameter / return-type enforcement — docs/known-issues.md §29.
//
// Every expectation here was probed against Lucee 7.0.4 first (the probes live
// in tests/functions/fn_type_enforcement_crossengine_probe*.cfm) and this suite
// passes on both engines. Two Lucee behaviours drive most of it:
//
//   * types are VALIDATED, never coerced — a `numeric` param given "123"
//     receives the string "123", unchanged;
//   * a type name Lucee has no cast target for is treated as a component path,
//     so it rejects every value — `integer`, `float`, `double`, `email` and
//     friends throw even for values that are obviously of that type.
//
// Cases deliberately NOT asserted here, because the two engines disagree for
// reasons that are not about type enforcement: anything involving a date VALUE
// (RustCFML dates are strings, Lucee's are DateTime objects, so the two name
// the offending value differently), a query column (an Array here, a scalar
// there), and the synthesized name of a closure in a message.

suiteBegin( "Function declared-type enforcement (§29)" );

// A throwing call, described by what came out: "<type>|<message>".
function typeFailure( cb ) {
    try {
        cb();
        return "NO THROW";
    } catch ( any e ) {
        return e.type & "|" & e.message;
    }
}

// ---------------------------------------------------------------- arguments
function argNumeric( numeric n ) { return n; }

assert( "numeric rejects a non-numeric string",
    typeFailure( function(){ return argNumeric( "abc" ); } ),
    "expression|Invalid call of the function [argNumeric], first Argument [n] is of invalid type, Cannot cast String [abc] to a value of type [numeric]" );

// Validation, not coercion: the string arrives as a string.
assert( "numeric accepts a numeric string, unconverted", argNumeric( "123" ), "123" );
assert( "numeric accepts surrounding whitespace", argNumeric( " 42 " ), " 42 " );
assert( "numeric accepts exponent notation", argNumeric( "1e3" ), "1e3" );
assert( "numeric accepts a leading sign", argNumeric( "+5" ), "+5" );
assert( "numeric accepts a bare fraction", argNumeric( ".5" ), ".5" );
assert( "numeric accepts a boolean", argNumeric( true ), true );
assertTrue( "numeric rejects the empty string",
    typeFailure( function(){ return argNumeric( "" ); } ) contains "Cannot cast String [] to a value of type [numeric]" );
assertTrue( "numeric rejects a thousands separator",
    typeFailure( function(){ return argNumeric( "1,000" ); } ) contains "Cannot cast String [1,000]" );
assertTrue( "numeric rejects hex notation",
    typeFailure( function(){ return argNumeric( "0x10" ); } ) contains "Cannot cast String [0x10]" );
assertTrue( "numeric rejects a trailing unit",
    typeFailure( function(){ return argNumeric( "5px" ); } ) contains "Cannot cast String [5px]" );
assert( "numeric rejects an array, named as an Object type",
    typeFailure( function(){ return argNumeric( [] ); } ),
    "expression|Invalid call of the function [argNumeric], first Argument [n] is of invalid type, Cannot cast Object type [Array] to a value of type [numeric]" );
assertTrue( "numeric rejects a struct",
    typeFailure( function(){ return argNumeric( {} ) ; } ) contains "Cannot cast Object type [Struct] to a value of type [numeric]" );

function argString( string s ) { return s; }
assert( "string accepts a number", argString( 123 ), 123 );
assert( "string accepts a boolean", argString( true ), true );
assertTrue( "string rejects an array",
    typeFailure( function(){ return argString( [] ); } ) contains "Cannot cast Object type [Array] to a value of type [string]" );
assertTrue( "string rejects a struct",
    typeFailure( function(){ return argString( {} ); } ) contains "Cannot cast Object type [Struct] to a value of type [string]" );

function argBoolean( boolean b ) { return b; }
assert( "boolean accepts yes", argBoolean( "yes" ), "yes" );
assert( "boolean accepts NO in any case", argBoolean( "NO" ), "NO" );
assert( "boolean accepts any number", argBoolean( 2 ), 2 );
assert( "boolean accepts a numeric string", argBoolean( "1.5" ), "1.5" );
assertTrue( "boolean rejects a non-boolean word",
    typeFailure( function(){ return argBoolean( "abc" ); } ) contains "Cannot cast String [abc] to a value of type [boolean]" );
assertTrue( "boolean rejects the empty string",
    typeFailure( function(){ return argBoolean( "" ); } ) contains "Cannot cast String [] to a value of type [boolean]" );

function argDate( date d ) { return "ok"; }
assert( "date accepts an ISO date", argDate( "2020-01-02" ), "ok" );
assert( "date accepts slash-separated ISO order", argDate( "2020/1/2" ), "ok" );
assert( "date accepts a US date", argDate( "1/2/2020" ), "ok" );
assert( "date accepts a month name", argDate( "January 2, 2020" ), "ok" );
assert( "date accepts a time of day", argDate( "10:30" ), "ok" );
assert( "date accepts an ODBC literal", argDate( "{ts '2020-01-02 00:00:00'}" ), "ok" );
assert( "date accepts a numeric serial", argDate( 0 ), "ok" );
assert( "date accepts a numeric string", argDate( "1" ), "ok" );
assertTrue( "date rejects an unparseable string",
    typeFailure( function(){ return argDate( "not a date" ); } ) contains "Cannot cast String [not a date] to a value of type [date]" );
assertTrue( "date rejects an impossible date",
    typeFailure( function(){ return argDate( "13/13/2020" ); } ) contains "Cannot cast String [13/13/2020]" );

function argArray( array a ) { return "ok"; }
assert( "array accepts an array", argArray( [ 1 ] ), "ok" );
// A struct with no non-numeric keys casts to an array; one with a named key
// does not. The value still arrives as a struct either way.
assert( "array accepts an empty struct", argArray( {} ), "ok" );
assert( "array accepts a numerically-keyed struct", argArray( { "1" : "a" } ), "ok" );
assertTrue( "array rejects a named-key struct",
    typeFailure( function(){ return argArray( { a : 1 } ); } ) contains "Cannot cast Object type [Struct] to a value of type [array]" );
assertTrue( "array rejects a list string",
    typeFailure( function(){ return argArray( "a,b" ); } ) contains "Cannot cast String [a,b] to a value of type [array]" );

function argStruct( struct s ) { return "ok"; }
assert( "struct accepts a struct", argStruct( { a : 1 } ), "ok" );
assert( "struct accepts a component instance",
    argStruct( createObject( "component", "TypeEnforcementFixture" ) ), "ok" );
assertTrue( "struct rejects an array",
    typeFailure( function(){ return argStruct( [] ); } ) contains "Cannot cast Object type [Array] to a value of type [struct]" );

function argQuery( query q ) { return "ok"; }
assert( "query accepts a query", argQuery( queryNew( "a" ) ), "ok" );
assertTrue( "query rejects an array",
    typeFailure( function(){ return argQuery( [] ); } ) contains "Cannot cast Object type [Array] to a value of type [query]" );

function argXml( xml x ) { return "ok"; }
assert( "xml accepts a document string", argXml( "<a>x</a>" ), "ok" );
assert( "xml accepts a self-closing element", argXml( "<a/>" ), "ok" );
assertTrue( "xml rejects an unclosed element",
    typeFailure( function(){ return argXml( "<a>" ); } ) contains "Cannot cast String [<a>] to a value of type [xml]" );
assertTrue( "xml rejects a plain string",
    typeFailure( function(){ return argXml( "x" ); } ) contains "Cannot cast String [x] to a value of type [xml]" );

function argFunction( function f ) { return "ok"; }
assert( "function accepts a closure", argFunction( function(){ return 1; } ), "ok" );
assertTrue( "function rejects a string",
    typeFailure( function(){ return argFunction( "x" ); } ) contains "Cannot cast String [x] to a value of type [function]" );

function argUuid( uuid u ) { return "ok"; }
assert( "uuid accepts createUUID()", argUuid( createUUID() ), "ok" );
assertTrue( "uuid rejects a non-uuid string",
    typeFailure( function(){ return argUuid( "nope" ); } ) contains "Cannot cast String [nope] to a value of type [uuid]" );
assertTrue( "uuid rejects the GUID layout",
    typeFailure( function(){ return argUuid( "6F9619FF-8B86-D011-B42D-00CF4FC964FF" ); } ) contains "to a value of type [uuid]" );

function argVariableName( variablename v ) { return "ok"; }
assert( "variablename accepts an identifier", argVariableName( "myVar_1" ), "ok" );
assertTrue( "variablename rejects a leading digit",
    typeFailure( function(){ return argVariableName( "1x" ); } ) contains "to a value of type [variablename]" );
assertTrue( "variablename rejects an embedded space",
    typeFailure( function(){ return argVariableName( "a b" ); } ) contains "to a value of type [variablename]" );

function argAny( any a ) { return "ok"; }
assert( "any accepts a struct", argAny( {} ), "ok" );
assert( "any accepts an array", argAny( [] ), "ok" );

// ------------------------------------------ type names with no cast target
// Lucee resolves these as component paths, so they reject EVERY value —
// including values that are plainly of that type. Mirrored deliberately.
function argInteger( integer i ) { return "ok"; }
assertTrue( "integer rejects an integer (Lucee has no integer cast target)",
    typeFailure( function(){ return argInteger( 5 ); } ) contains "Cannot cast Object type [Number] to a value of type [integer]" );
assertTrue( "integer rejects an integral string",
    typeFailure( function(){ return argInteger( "5" ); } ) contains "Cannot cast String [5] to a value of type [integer]" );
function argFloat( float f ) { return "ok"; }
assertTrue( "float rejects a float",
    typeFailure( function(){ return argFloat( 1.5 ); } ) contains "to a value of type [float]" );
function argEmail( email e ) { return "ok"; }
assertTrue( "email rejects a valid email address",
    typeFailure( function(){ return argEmail( "a@b.com" ); } ) contains "Cannot cast String [a@b.com] to a value of type [email]" );
function argWidget( widget w ) { return "ok"; }
assertTrue( "an unknown type name rejects a string",
    typeFailure( function(){ return argWidget( "x" ); } ) contains "Cannot cast String [x] to a value of type [widget]" );
assertTrue( "an unknown type name rejects a struct",
    typeFailure( function(){ return argWidget( {} ); } ) contains "Cannot cast Object type [Struct] to a value of type [widget]" );

// ------------------------------------------------------------- the ordinal
// Lucee spells out the first two positions and then goes numeric — including
// the ungrammatical "3th", which is its wording, not a typo.
function argThree( any a, numeric b, string c ) { return "ok"; }
assertTrue( "the second argument is named 'second'",
    typeFailure( function(){ return argThree( 1, "abc", "s" ); } ) contains "second Argument [b] is of invalid type" );
assertTrue( "the third argument is named '3th'",
    typeFailure( function(){ return argThree( 1, 2, [] ); } ) contains "3th Argument [c] is of invalid type" );

// ------------------------------------------------- how the argument arrives
// Named and argumentCollection calls are checked exactly like positional ones.
assertTrue( "a named argument is checked",
    typeFailure( function(){ return argNumeric( n = "abc" ); } ) contains "first Argument [n] is of invalid type" );
assertTrue( "an argumentCollection argument is checked",
    typeFailure( function(){ return argNumeric( argumentCollection = { n : "abc" } ); } ) contains "first Argument [n] is of invalid type" );

// A DEFAULT is checked too, but only when it is actually applied.
function argDefaultBad( numeric n = "abc" ) { return n; }
assertTrue( "an applied default that violates the type throws",
    typeFailure( function(){ return argDefaultBad(); } ) contains "Cannot cast String [abc] to a value of type [numeric]" );
assert( "supplying a valid argument skips the bad default", argDefaultBad( 7 ), 7 );
function argDefaultOk( numeric n = "7" ) { return n; }
assert( "an applied default that satisfies the type passes", argDefaultOk(), "7" );

// An omitted optional argument is absent, not Null-checked.
function argOptional( numeric n ) { return isNull( n ) ? "absent" : n; }
assert( "an omitted optional argument is not type-checked", argOptional(), "absent" );

// ---------------------------------------------------------- typed arrays
function argStringArray( string[] v ) { return "ok"; }
assert( "string[] accepts an empty array", argStringArray( [] ), "ok" );
assert( "string[] accepts strings", argStringArray( [ "a", "b" ] ), "ok" );
assert( "string[] accepts numbers (they are valid strings)", argStringArray( [ 1, 2 ] ), "ok" );
assertTrue( "string[] rejects a nested array element",
    typeFailure( function(){ return argStringArray( [ "a", [] ] ); } ) contains "Cannot cast Object type [Array] to a value of type [string[]]" );
assertTrue( "string[] rejects a struct",
    typeFailure( function(){ return argStringArray( {} ); } ) contains "to a value of type [string[]]" );
function argNumericArray( numeric[] v ) { return "ok"; }
assert( "numeric[] accepts numeric strings", argNumericArray( [ "5" ] ), "ok" );
assertTrue( "numeric[] rejects a non-numeric element",
    typeFailure( function(){ return argNumericArray( [ "x" ] ); } ) contains "to a value of type [numeric[]]" );
function argNestedArray( string[][] v ) { return "ok"; }
assert( "string[][] accepts an array of string arrays", argNestedArray( [ [ "a" ] ] ), "ok" );
assertTrue( "string[][] rejects a flat array",
    typeFailure( function(){ return argNestedArray( [ "a" ] ); } ) contains "to a value of type [string[][]]" );

// ------------------------------------------------------------ return types
function retNumericBad() returntype="numeric" { return "abc"; }
// Lucee has two message forms and picks by the VALUE: a String gets the bare
// cast error, anything else gets it wrapped in "has an invalid return value".
assert( "a bad string return is reported bare",
    typeFailure( function(){ return retNumericBad(); } ),
    "expression|Cannot cast String [abc] to a value of type [numeric]" );
function retStringBad() returntype="string" { return []; }
assert( "a bad non-string return is reported wrapped",
    typeFailure( function(){ return retStringBad(); } ),
    "expression|The function [retStringBad] has an invalid return value , [Cannot cast Object type [Array] to a value of type [string]]" );
function retNumericOk() returntype="numeric" { return "42"; }
assert( "a return value is validated, not converted", retNumericOk(), "42" );
function retDateBad() returntype="date" { return "nope"; }
// In return position the type is named canonically — `date` reports as `datetime`.
assert( "a date return type is named datetime",
    typeFailure( function(){ return retDateBad(); } ),
    "expression|Cannot cast String [nope] to a value of type [datetime]" );
function retVoidBad() returntype="void" { return 5; }
assertTrue( "returning a value from a void function throws",
    typeFailure( function(){ return retVoidBad(); } ) contains "Cannot cast Object type [Number] to a value of type [void]" );
function retVoidOk() returntype="void" { }
assert( "returning nothing from a void function is fine",
    isNull( retVoidOk() ) ? "null" : "notnull", "null" );
function retNumericNothing() returntype="numeric" { }
assert( "falling off the end of a typed function returns null, unchecked",
    isNull( retNumericNothing() ) ? "null" : "notnull", "null" );
function retAny() returntype="any" { return {}; }
assert( "an any return type is never checked", isStruct( retAny() ), true );

// The prefix form of the declaration is enforced the same way.
numeric function retPrefixBad() { return "abc"; }
assertTrue( "a prefix-declared return type is enforced",
    typeFailure( function(){ return retPrefixBad(); } ) contains "Cannot cast String [abc] to a value of type [numeric]" );

// ------------------------------------------------- components and CFC tags
// NB the fixture is re-created inside each callback rather than captured from
// a page-scope variable: RustCFML cannot currently see a page-scope variable
// from inside a closure defined later on the page (unrelated to §29, but it
// would make this suite fail here for the wrong reason).
function fixture() { return createObject( "component", "TypeEnforcementFixture" ); }
assertTrue( "a tag-declared argument type is enforced",
    typeFailure( function(){ return fixture().tagArg( "abc" ); } ) contains "Invalid call of the function [tagArg], first Argument [n] is of invalid type, Cannot cast String [abc] to a value of type [numeric]" );
assert( "a tag-declared argument type accepts a valid value", fixture().tagArg( "7" ), "7" );
assertTrue( "a tag-declared return type is enforced",
    typeFailure( function(){ return fixture().tagRet(); } ) contains "Cannot cast String [abc] to a value of type [numeric]" );
assert( "a tag-declared return type accepts a valid value", fixture().tagRetOk(), "42" );
assert( "a component-typed argument accepts an instance",
    fixture().componentArg( fixture() ), "accepted" );
assertTrue( "a component-typed argument rejects a plain struct",
    typeFailure( function(){ return fixture().componentArg( {} ); } ) contains "Cannot cast Object type [Struct] to a value of type [TypeEnforcementFixture]" );

suiteEnd();
</cfscript>
