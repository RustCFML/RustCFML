<cfscript>
// GH ##549: a closure must keep its own captured variables when called from
// another closure that held it via a function local or argument. Stripping
// captured_scope to break env→fn→env cycles is only correct for a SAME-env
// self-reference; a foreign closure's env must survive the capture.
suiteBegin( "Nested closure preserves foreign captured scope (GH ##549)" );

function makeCounter() {
	var n = 0;
	return function() {
		n++;
		return n;
	};
}

// Shape 1: called directly
c = makeCounter();
assert( "direct first call", c(), 1 );
assert( "direct second call", c(), 2 );

// Shape 2: from a closure defined at page level
c2 = makeCounter();
wrapper = function() {
	return c2();
};
assert( "page-level wrapper first", wrapper(), 1 );
assert( "page-level wrapper second", wrapper(), 2 );

// Shape 3: from a closure that captured it from a function argument
function makeSharedArg( required any counter ) {
	return function() {
		return counter();
	};
}
s3 = makeSharedArg( makeCounter() );
assert( "argument-held first", s3(), 1 );
assert( "argument-held second", s3(), 2 );

// Shape 4: from a closure that captured it from a function local
function makeShared() {
	var counter = makeCounter();
	return function() {
		return counter();
	};
}
s4 = makeShared();
assert( "local-held first", s4(), 1 );
assert( "local-held second", s4(), 2 );

suiteEnd();


suiteBegin( "Deeper nesting and shared mutable state (GH ##549)" );

// Three factory levels: each level captures the inner closure from a local
function level1() {
	var c = makeCounter();
	return function() {
		return c();
	};
}
function level2() {
	var inner = level1();
	return function() {
		return inner();
	};
}
function level3() {
	var inner = level2();
	return function() {
		return inner();
	};
}
t = level3();
assert( "three-level first", t(), 1 );
assert( "three-level second", t(), 2 );
assert( "three-level third", t(), 3 );

// Closure returned via a struct from a local
function makeInStruct() {
	var counter = makeCounter();
	var box = { fn: counter };
	return function() {
		return box.fn();
	};
}
s = makeInStruct();
assert( "struct-held first", s(), 1 );
assert( "struct-held second", s(), 2 );

// Closure returned via an array from an argument
function makeInArray( required any counter ) {
	var bag = [ counter ];
	return function() {
		var f = bag[ 1 ];
		return f();
	};
}
a = makeInArray( makeCounter() );
assert( "array-held first", a(), 1 );
assert( "array-held second", a(), 2 );

// Two wrappers share one counter — mutations are visible across both
function makeSharedPair() {
	var counter = makeCounter();
	var left = function() {
		return counter();
	};
	var right = function() {
		return counter();
	};
	return { left: left, right: right };
}
p = makeSharedPair();
assert( "shared pair left first", p.left(), 1 );
assert( "shared pair right sees left's write", p.right(), 2 );
assert( "shared pair left sees right's write", p.left(), 3 );

// Argument-held chain of wrappers
function wrapOnce( required any inner ) {
	return function() {
		return inner();
	};
}
w = wrapOnce( wrapOnce( makeCounter() ) );
assert( "double-wrapped arg first", w(), 1 );
assert( "double-wrapped arg second", w(), 2 );

// Same-frame sibling helper (PR #198) must still resolve: stripping the
// SAME-env self-reference remains required for the leak guard.
function siblings() {
	var total = 0;
	var bump = function() {
		total++;
		return total;
	};
	var run = function() {
		return bump();
	};
	return run();
}
assert( "same-frame sibling helper still works", siblings(), 1 );
assert( "same-frame sibling helper second call is independent", siblings(), 1 );

suiteEnd();
</cfscript>
