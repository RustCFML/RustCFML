<cfscript>
suiteBegin("Reserved words as for-in loop variables");

// Lucee accepts ANY reserved word as a loop variable; RustCFML rejected them,
// and the parse error was then swallowed into an empty metadata struct (see the
// getComponentMetaData assertions below), so the real failure surfaced far away
// as ColdBox's "Variable 'name' is undefined" during application boot.
q = QueryNew( "label", "varchar", [ [ "one" ], [ "two" ] ] );

cases = createObject( "component", "core.kwloop.CaseLoop" );
assert( "`for( var case in query )` iterates", cases.joinCases( q ), "one,two" );
assert( "other reserved words work as loop variables", cases.loopKeywords(), "abcde" );

// The same outside a component. (Wrapped in functions: `var` at page scope is
// an error on Lucee, so a page-level loop would abort the file there.)
function inlineVarLoop() {
	var out = "";
	for ( var case in [ "x", "y" ] ) { out &= case; }
	return out;
}
assert( "inline `for( var case in array )`", inlineVarLoop(), "xy" );

function inlineBareLoop() {
	var out = "";
	for ( case in [ "p", "q" ] ) { out &= case; }
	return out;
}
assert( "...and without `var`", inlineBareLoop(), "pq" );

// A reserved-word loop variable must not disturb the classic form.
function classicFor() {
	var total = 0;
	for ( var i = 1; i <= 3; i++ ) { total += i; }
	return total;
}
assert( "a classic for loop still parses", classicFor(), 6 );

// Metadata for such a component is real, not an empty struct.
md = getComponentMetaData( "core.kwloop.CaseLoop" );
assert( "metadata carries the component name", md.name ?: "", "core.kwloop.CaseLoop" );
assertTrue( "metadata carries its functions", ArrayLen( md.functions ?: [] ) >= 2 );

// And an unloadable component THROWS rather than answering with {}.
assertThrows( "getComponentMetaData on a missing component throws"
            , function() { getComponentMetaData( "no.such.ComponentAnywhere" ); } );

suiteEnd();
</cfscript>
