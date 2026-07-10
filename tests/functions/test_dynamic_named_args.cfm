<cfscript>
suiteBegin("Dynamic (computed) named arguments");

// A quoted/interpolated string as a parameter NAME resolves at runtime.
// Lucee supports `func( "#expr#" = value )`; Preside's FrontendEditingService
// uses exactly this: editPage( id=.., isDraft=true, "#property#"=content ).
function capture( id, isDraft ) {
	return arguments;
}

prop = "main_content";
content = "hello";

// interpolated name alongside static named args (the Preside case)
r1 = capture( id = "R1", isDraft = true, "#prop#" = content );
assert( "interpolated arg name resolves to its runtime value", r1.main_content, "hello" );
assert( "static named args still bind (id)", r1.id, "R1" );
assertTrue( "static named args still bind (isDraft)", r1.isDraft );

// colon separator form
r2 = capture( id = "R2", "#prop#" : content );
assert( "colon-separated dynamic name", r2.main_content, "hello" );

// pure literal-string name (no interpolation)
r3 = capture( id = "R3", "extra_field" = "lit" );
assert( "literal-string arg name", r3.extra_field, "lit" );

// all arguments dynamic
r4 = capture( "#prop#" = content );
assert( "all-dynamic arg name", r4.main_content, "hello" );

// still-good: all-static named args are unaffected
r5 = capture( id = "R5", isDraft = false );
assert( "static-only named args unaffected", r5.id, "R5" );

// mixing positional with a named arg is still an error (Lucee parity)
assertThrows( "mixed positional + named still throws", function() {
	capture( "R6", isDraft = true );
} );

suiteEnd();
</cfscript>
