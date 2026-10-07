<cfscript>
suiteBegin("Tag-in-script calls: whitespace-separated attributes");

// Lucee's `cfTag( attr=… attr=… )` call form takes a COMMA or plain whitespace
// between attributes. A shipped Preside extension (preside-ext-zoom-apis'
// cf_jwt.cfc) writes `cfthrow( type="Invalid Token" message="…" )`, and
// rejecting it took the whole component down with a parse error. The licence is
// specific to the cfXXX() aliases: Lucee fails to compile a UDF or BIF call
// written that way, so this file only exercises tag calls.

function commaless() {
	try { cfthrow( type="Invalid Token" message="Token should contain 3 segments" ); }
	catch( any e ) { return e.type & "/" & e.message; }
}
assert( "whitespace alone separates cfthrow attributes"
      , commaless(), "Invalid Token/Token should contain 3 segments" );

function mixedSeparators() {
	try { cfthrow( type="A", message="B" detail="C" ); }
	catch( any e ) { return e.type & "/" & e.message & "/" & e.detail; }
}
assert( "commas and whitespace mix freely", mixedSeparators(), "A/B/C" );

// The attribute VALUE is a full expression, so the parser has to stop at the
// next `name=` binding rather than swallowing it.
function expressionValue() {
	try { cfthrow( type="A" & "Z" message="B" ); }
	catch( any e ) { return e.type & "/" & e.message; }
}
assert( "an expression value ends at the next attribute", expressionValue(), "AZ/B" );

function allCommas() {
	try { cfthrow( type="A", message="B" ); } catch( any e ) { return e.type & "/" & e.message; }
}
assert( "the all-comma form is unaffected", allCommas(), "A/B" );

// Not just cfthrow — every tag call takes the same form.
function readWithTagCall() {
	cffile( action="read" file=getCurrentTemplatePath() variable="local.contents" );
	return len( local.contents ) > 0;
}
assertTrue( "cffile() reads with whitespace-separated attributes", readWithTagCall() );

function dumpCall() {
	savecontent variable="local.out" { cfdump( var=[ 1, 2 ] label="L" ); }
	return len( local.out ) > 0;
}
assertTrue( "cfdump() renders with whitespace-separated attributes", dumpCall() );

suiteEnd();
</cfscript>
