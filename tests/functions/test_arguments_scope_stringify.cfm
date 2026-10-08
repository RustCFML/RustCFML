<cfscript>
// The arguments scope carries two internal bookkeeping keys —
// `__arguments_scope` (the sentinel) and `__arguments_params` (the declared
// param names, backing positional `arguments[i]`). Every CFML-visible view of
// a struct hides them, and STRINGIFYING one must too.
//
// It did not: `arguments.toString()` dumped the raw backing map. ColdBox's
// WireBox Builder captures an injection definition as an arguments scope and
// puts it straight into its DSLDependencyNotFoundException message, so a
// Preside boot error read
//   {__arguments_params: [name, ref, dsl, ...], __arguments_scope: true, ...}
// in front of the user.
suiteBegin("Arguments scope stringification");

function capture( required string name, string dsl="", boolean required=true ) {
	return arguments;
}
d = capture( name="oauthProxyConnectionService", dsl="OauthProxyConnectionService" );

assertFalse( "toString() hides __arguments_scope" , d.toString() contains "__arguments_scope"  );
assertFalse( "toString() hides __arguments_params", d.toString() contains "__arguments_params" );
assertTrue ( "toString() keeps the real keys"     , d.toString() contains "oauthProxyConnectionService" );

// The views that were already correct must stay correct.
assert( "structKeyList unchanged", structKeyList( d ), "name,dsl,required" );
assert( "structCount unchanged"  , structCount( d )  , 3 );
assertFalse( "serializeJson hides the markers", serializeJson( d ) contains "__arguments_" );

// An arguments scope and a plain struct with the same content must stringify
// identically — TestBox/MockBox hashes this string to match `$args( {...} )`
// against the struct the subject under test builds.
function echoArgs( a, b ) { return arguments; }
assert(
	  "an arguments scope stringifies like the equivalent struct"
	, echoArgs( a=1, b=2 ).toString()
	, { a=1, b=2 }.toString()
);

suiteEnd();
</cfscript>
