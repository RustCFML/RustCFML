<cfscript>
// Call-dispatch Lever C: the eagerly-built `arguments` scope is allocated
// UNTRACKED (skips cycle-GC logging) only when a static bytecode scan proves it
// cannot escape the call frame — i.e. it is referenced solely via `arguments.foo`
// member reads. Every other shape (bare `arguments`, `arguments[N]`,
// argumentCollection forwarding, closure capture, structKeyExists/isDefined,
// includes/cfmodule) stays TRACKED. Untracking is pure GC bookkeeping, so
// behaviour MUST be byte-identical either way. These tests lock that in and
// exercise both classifier branches.
suiteBegin("Arguments scope (call-dispatch Lever C)");

// --- UNTRACKED path: pure `arguments.foo` member reads --------------------
function sumMember( a, b ) {
	return arguments.a + arguments.b;
}
assert( "arguments.foo read (untracked path)", sumMember( 3, 4 ), 7 );

// mutate a param, then read it back through the arguments scope
function mutateThenRead( x ) {
	arguments.x = arguments.x * 10;
	return arguments.x;
}
assert( "mutate arg then read via arguments.foo", mutateThenRead( 5 ), 50 );

// named-arg binding still surfaces under the param name
assert( "named args bind by name (untracked path)", sumMember( b = 20, a = 2 ), 22 );

// recursion: each call's arguments scope is independent (untracked is per-call)
function fact( n ) {
	if ( arguments.n <= 1 ) {
		return 1;
	}
	return arguments.n * fact( arguments.n - 1 );
}
assert( "recursive arguments.foo independent per call", fact( 5 ), 120 );

// --- TRACKED path: bare `arguments` returned (escapes via return) ---------
function echoArgs( id, isDraft ) {
	return arguments;
}
r = echoArgs( id = "R1", isDraft = true );
assert( "returned arguments struct carries id", r.id, "R1" );
assertTrue( "returned arguments struct carries isDraft", r.isDraft );

// two independent returned scopes must not alias each other
a1 = echoArgs( id = "A", isDraft = false );
a2 = echoArgs( id = "B", isDraft = true );
assert( "first returned scope unchanged after second", a1.id, "A" );
assert( "second returned scope distinct", a2.id, "B" );

// --- TRACKED path: argumentCollection forwarding --------------------------
function forwardee( first, second ) {
	return arguments.first & "/" & arguments.second;
}
function forwarder( first, second ) {
	return forwardee( argumentCollection = arguments );
}
assert( "argumentCollection=arguments forwarding", forwarder( "x", "y" ), "x/y" );

// --- TRACKED path: closure capturing the enclosing arguments --------------
function makeReader( label ) {
	var captured = arguments;          // bare load + store => escapes => tracked
	return function() {
		return captured.label;
	};
}
reader = makeReader( label = "kept" );
assert( "closure sees captured enclosing arguments", reader(), "kept" );

// --- TRACKED path: positional arguments[N] --------------------------------
function positional() {
	return arguments[ 1 ] & arguments[ 2 ];
}
assert( "positional arguments[N] (overflow, tracked)", positional( "p", "q" ), "pq" );

// --- TRACKED path: introspection over the whole scope ---------------------
function introspect( only ) {
	return structKeyExists( arguments, "only" ) & "," & arrayLen( arguments );
}
assert( "structKeyExists + arrayLen over arguments", introspect( "z" ), "true,1" );

suiteEnd();
</cfscript>
