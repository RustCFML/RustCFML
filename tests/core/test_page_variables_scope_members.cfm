<cfscript>
suiteBegin( "The page variables scope lists only the page's own variables (Lucee parity)" );
// Every builtin, the engine's `__` keys, a component template awaiting its
// finalize and the other scopes (`cgi`, `url`, `form`) lived in the page's
// globals map and surfaced as members of `variables`: `structCount(variables)`
// was 754 on a page with three variables. Worse, `variables.x = 1` round-
// tripped that view and spliced all of them back into the page frame, after
// which every UDF call from the page seeded ~750 keys into its frame
// (800 ns → 59 µs per call). Expectations verified on Lucee 7.1.
function pvUdf( x ) { return x + 1; }
pvA = 1; pvB = "two";
before = structKeyList( variables );
assertTrue( "own variables are members", listFindNoCase( before, "pvA" ) && listFindNoCase( before, "pvB" ) );
assertTrue( "a page UDF is a member", listFindNoCase( before, "pvUdf" ) > 0 );
assertFalse( "a builtin is not a member", structKeyExists( variables, "arrayLen" ) );
assertFalse( "isDefined does not see a builtin through variables", isDefined( "variables.arrayLen" ) );
assertFalse( "the cgi scope is not a member of variables", structKeyExists( variables, "cgi" ) );
assertFalse( "no engine __ keys", arrayLen( structKeyArray( variables ).filter( function( k ) { return left( k, 2 ) == "__"; } ) ) > 0 );
variables.pvC = 3;
assertTrue( "a scoped write adds exactly its key", structKeyExists( variables, "pvC" ) );
// `before` itself became a member, so the growth is two keys: before + pvC.
assert( "a scoped write does not grow the scope by the builtin count", structCount( variables ) - listLen( before ), 2 );
assert( "call after a scoped write still works", pvUdf( 1 ), 2 );
suiteEnd();
</cfscript>
