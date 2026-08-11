<cfscript>
// Writes through the explicit `local.` scope prefix (increment / decrement /
// compound assign / nested paths). These lower to the same frame-key ops as a
// plain `local.x = v` assignment (perf plan T3.1 stage 1.5) instead of
// materializing and merging the whole `local` scope view — every expectation
// here was verified against Lucee 7.0.4.
suiteBegin( "local. scope member writes" );

function t_incdec() {
	local.i = 5;
	local.i++;
	++local.i;
	local.i--;
	return local.i;
}
assert( "local.i++ / ++local.i / local.i-- as statements", t_incdec(), 6 );

// A never-assigned `local.x` read is an error, exactly as in Lucee
// ("key [I] doesn't exist").
function t_undefined() {
	local.i++;
	return local.i;
}
assertThrows( "local.i++ with no prior local.i throws", t_undefined );

// The increment must NOT leak the key into the caller (classic localMode
// write-back covers unscoped assignments only, never `local.`).
function t_leak_inner() {
	local.leaky = 1;
	local.leaky++;
	return local.leaky;
}
function t_leak_outer() {
	var r = t_leak_inner();
	return [ r, structKeyExists( variables, "leaky" ), structKeyExists( local, "leaky" ) ];
}
assert( "local.x++ does not leak the key to the caller", serializeJSON( t_leak_outer() ), serializeJSON( [ 2, false, false ] ) );

function t_compound() {
	local.x = 10;
	local.x += 5;
	local.x -= 2;
	local.x *= 3;
	local.x /= 2;
	local.s = "a";
	local.s &= "b";
	return [ local.x, local.s ];
}
assert( "compound assigns on local.x", serializeJSON( t_compound() ), serializeJSON( [ 19.5, "ab" ] ) );

function t_nested() {
	local.a = { b = 1, s = { c = 2 } };
	local.a.b++;
	local.a.s.c += 5;
	return local.a;
}
nested = t_nested();
assert( "nested local.a.b++", nested.b, 2 );
assert( "nested local.a.s.c += K", nested.s.c, 7 );

function t_forloop() {
	local.total = 0;
	for( local.i = 1; local.i <= 4; local.i++ ) {
		local.total += local.i;
	}
	return [ local.i, local.total ];
}
assert( "for loop with a local. counter", serializeJSON( t_forloop() ), serializeJSON( [ 5, 10 ] ) );

function t_rvalue() {
	local.i = 1;
	local.post = local.i++;
	local.pre = ++local.i;
	return [ local.post, local.pre, local.i ];
}
assert( "local.i++ / ++local.i in rvalue position", serializeJSON( t_rvalue() ), serializeJSON( [ 1, 3, 3 ] ) );

// `local.i` is its own key — writing it never touches a same-named argument.
function t_param_shadow( i ) {
	local.i = 100;
	local.i++;
	return [ local.i, arguments.i, i ];
}
assert( "local.i is distinct from a same-named argument", serializeJSON( t_param_shadow( 7 ) ), serializeJSON( [ 101, 7, 101 ] ) );

// …and a same-named argument does not satisfy a `local.i` read.
function t_param_only( i ) {
	local.i++;
	return local.i;
}
assertThrows( "an argument does not make local.i defined", function() { t_param_only( 7 ); } );

// Nor does an inherited caller key.
function t_inherited_inner() {
	local.i++;
	return local.i;
}
function t_inherited_outer() {
	variables.i = 100;
	return t_inherited_inner();
}
assertThrows( "a caller's variables.i does not make local.i defined", t_inherited_outer );

function t_array() {
	local.arr = [ 1, 2, 3 ];
	local.arr[ 2 ]++;
	local.arr[ 3 ] += 10;
	return local.arr;
}
assert( "local.arr[i]++ / += K", serializeJSON( t_array() ), serializeJSON( [ 1, 3, 13 ] ) );

// A closure in the body makes the frame slot-ineligible; the answers must not
// depend on that.
function t_with_closure() {
	local.i = 1;
	local.f = function() { return 42; };
	local.i++;
	return [ local.i, local.f() ];
}
assert( "same answers in a closure-defining (slot-ineligible) frame", serializeJSON( t_with_closure() ), serializeJSON( [ 2, 42 ] ) );

function t_unscoped_read() {
	local.i = 3;
	local.i++;
	return [ i, local.i ];
}
assert( "unscoped read sees the local.-prefixed increment", serializeJSON( t_unscoped_read() ), serializeJSON( [ 4, 4 ] ) );

suiteEnd();
</cfscript>
