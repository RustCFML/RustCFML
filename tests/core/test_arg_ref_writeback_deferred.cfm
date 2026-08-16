<cfscript>
/*
 * Pass-by-reference argument write-back, pinned after the phase-8 change that
 * DEFERRED `arg_sources_cached` out of the hot call prologue and into the
 * write-back branch itself.
 *
 * The lookup maps a call-site argument position back to the caller's local
 * variable name, and it used to run on every single call (~42 ns each, ~0.53 ms
 * per warm Preside render) to serve a branch that fires on well under 1% of
 * them. Deferring it is only safe if the call-site instruction pointer captured
 * BEFORE the call still resolves correctly afterwards — including down the
 * error path, which reassigns `ip` to the catch handler. These assertions are
 * what would go quiet if that capture were ever wrong: the mutation simply
 * stops propagating to the caller and nothing throws.
 */
suiteBegin( "Arg by-ref write-back (deferred arg_sources)" );

// --- positional calls -------------------------------------------------------

function mutateStruct( s ) {
	s.touched = "yes";
	return "done";
}

function mutateArray( a ) {
	arrayAppend( a, "added" );
	return "done";
}

holder = { start = 1 };
mutateStruct( holder );
assert( "positional: struct mutation propagates to caller local", holder.touched ?: "MISSING", "yes" );
assert( "positional: struct keeps its original key", holder.start, 1 );

list = [ "a" ];
mutateArray( list );
assert( "positional: array mutation propagates", arrayLen( list ), 2 );
assert( "positional: appended value is correct", list[ 2 ], "added" );

// Two different caller locals through the SAME callee: each call site must map
// back to its own variable, which is exactly what arg_sources resolves.
first = { n = 1 };
second = { n = 2 };
mutateStruct( first );
mutateStruct( second );
assert( "positional: first local mutated", first.touched ?: "MISSING", "yes" );
assert( "positional: second local mutated", second.touched ?: "MISSING", "yes" );

// Multiple args — the write-back indexes by parameter position.
function mutateBoth( x, y ) {
	x.tag = "X";
	y.tag = "Y";
	return "";
}
left = {};
right = {};
mutateBoth( left, right );
assert( "positional: arg 1 mapped to its own local", left.tag ?: "MISSING", "X" );
assert( "positional: arg 2 mapped to its own local", right.tag ?: "MISSING", "Y" );

// --- named calls ------------------------------------------------------------

function mutateNamed( required struct target, string label = "n" ) {
	target.label = label;
	return "";
}

namedHolder = {};
mutateNamed( target = namedHolder, label = "hello" );
assert( "named: mutation propagates to caller local", namedHolder.label ?: "MISSING", "hello" );

// Named args supplied OUT of declaration order — the write-back has to walk the
// name list to find which call-site index fed the mutated parameter.
outOfOrder = {};
mutateNamed( label = "reordered", target = outOfOrder );
assert( "named: out-of-order args still map back", outOfOrder.label ?: "MISSING", "reordered" );

// --- the error path ---------------------------------------------------------
// A throwing callee sends `ip` to the catch handler. The deferred lookup uses
// the call-site ip captured before dispatch, so a later successful call in the
// same frame must still resolve correctly.

function mutateThenThrow( s ) {
	s.before = "set";
	throw( type = "RefTest", message = "boom" );
}

afterThrow = {};
caught = "";
try {
	mutateThenThrow( afterThrow );
} catch ( RefTest e ) {
	caught = e.message;
}
assert( "error path: exception still propagates", caught, "boom" );

recovered = {};
mutateStruct( recovered );
assert( "error path: a later call in the same frame still writes back", recovered.touched ?: "MISSING", "yes" );

// --- nested frames ----------------------------------------------------------

function outerMutator( s ) {
	mutateStruct( s );
	s.outer = "ran";
	return "";
}

nested = {};
outerMutator( nested );
assert( "nested: inner callee's mutation reaches the outermost caller", nested.touched ?: "MISSING", "yes" );
assert( "nested: outer frame's own mutation survives", nested.outer ?: "MISSING", "ran" );

// --- a non-variable argument expression -------------------------------------
// The argument is not a plain local, so there is no source variable to write
// back to; this must be a clean no-op rather than a mis-resolved write.
wrapper = { inner = {} };
mutateStruct( wrapper.inner );
assert( "member-expression arg still mutates the referenced struct", wrapper.inner.touched ?: "MISSING", "yes" );

literalResult = mutateStruct( {} );
assert( "literal arg does not disturb the call result", literalResult, "done" );

suiteEnd();
</cfscript>
