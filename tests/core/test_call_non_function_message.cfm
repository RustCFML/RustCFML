<cfscript>
// Calling a variable that holds data, not a function, reports the variable's
// name (GH #516). RustCFML said "Variable is not a function or function
// '<unknown>' is not defined"; Lucee 7.1 says "No matching function [X] found",
// and evaluates the arguments before it fails.
suiteBegin("Calling a non-function variable names it (GH 516)");

function cnf516Msg( required any fn ) {
	try {
		arguments.fn();
	} catch ( any e ) {
		return e.type & "|" & e.message;
	}
	return "no error";
}

function cnf516Target() { return "fn"; }
function cnf516LocalShadow() {
	var cnf516Target = { a = 1 };
	return cnf516Target();
}
function cnf516StringLocal() {
	var cnf516S = "str";
	return cnf516S();
}
function cnf516Param( x ) { return x( 1 ); }
function cnf516NullLocal() {
	var cnf516N = javacast( "null", "" );
	return cnf516N();
}
function cnf516Named() {
	var cnf516V = { a = 1 };
	return cnf516V( a = 1 );
}

request.cnf516Log = "";
function cnf516Arg() { request.cnf516Log &= "evaluated"; return 1; }
function cnf516ArgsFirst() {
	var cnf516W = { a = 1 };
	return cnf516W( cnf516Arg() );
}

assert( "local struct shadowing a UDF",
	cnf516Msg( cnf516LocalShadow ), "expression|No matching function [CNF516TARGET] found" );
assert( "local string",
	cnf516Msg( cnf516StringLocal ), "expression|No matching function [CNF516S] found" );
assert( "argument holding a number",
	cnf516Msg( function() { return cnf516Param( 5 ); } ), "expression|No matching function [X] found" );
assert( "local holding null",
	cnf516Msg( cnf516NullLocal ), "expression|No matching function [CNF516N] found" );
assert( "named-argument call",
	cnf516Msg( cnf516Named ), "expression|No matching function [CNF516V] found" );
assert( "arguments evaluated before the failure",
	cnf516Msg( cnf516ArgsFirst ) & "|" & request.cnf516Log,
	"expression|No matching function [CNF516W] found|evaluated" );

// Page scope.
cnf516Struct = { a = 1 };
try {
	cnf516Struct();
	cnf516PageMsg = "no error";
} catch ( any e ) {
	cnf516PageMsg = e.type & "|" & e.message;
}
assert( "page-scope struct", cnf516PageMsg, "expression|No matching function [CNF516STRUCT] found" );

cnf516Str = "str";
try {
	cnf516Str();
	cnf516PageMsg = "no error";
} catch ( any e ) {
	cnf516PageMsg = e.type & "|" & e.message;
}
assert( "page-scope string", cnf516PageMsg, "expression|No matching function [CNF516STR] found" );

// The real function still runs when nothing shadows it.
assert( "unshadowed UDF still callable", cnf516Target(), "fn" );

structDelete( request, "cnf516Log" );
suiteEnd();
</cfscript>
