<cfscript>
suiteBegin("throw() with mixed named/positional arguments (RustCFML superset)");

// `throw( type="x", "the message" )` appears in a shipped Preside extension.
// Lucee refuses to COMPILE the file that contains it ("Invalid argument for
// function [ throw ], You can't mix named and unNamed arguments"), so the whole
// component is unloadable there. RustCFML deliberately accepts the file and
// raises at the call instead: every other function in the component keeps
// working, and the bad line still fails when it runs. That makes this file
// RustCFML-only — Lucee cannot even parse it.
// --- 3. throw() mixing named and positional arguments ----------------------
// Lucee COMPILES this and raises only when the line runs. Rejecting it at parse
// time killed the file; now the error arrives at the call, typed `expression`.
function mixedThrowArgs() {
	try {
		throw( type="my.type", "the message" );
		return "no error";
	} catch( any e ) {
		return e.type;
	}
}
assert( "mixing named and positional throw args fails at RUNTIME, not parse time"
      , mixedThrowArgs(), "expression" );

// All-named and all-positional throws keep working.
function namedThrow() {
	try { throw( type="named.type", message="m" ); } catch( any e ) { return e.type & "/" & e.message; }
}
assert( "an all-named throw is unaffected", namedThrow(), "named.type/m" );

function positionalThrow() {
	try { throw( "posmsg", "pos.type" ); } catch( any e ) { return e.type & "/" & e.message; }
}
assert( "an all-positional throw is unaffected", positionalThrow(), "pos.type/posmsg" );

suiteEnd();
</cfscript>
