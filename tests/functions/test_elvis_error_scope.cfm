<cfscript>
// Elvis (`?:`) ERROR SCOPE — GH #329.
//
// Lucee absorbs ANY exception raised while evaluating the LEFT operand of
// `?:`, not merely an undefined/null read. A defensive one-liner like
//
//     getBaseTagData( name, i ).attributes.marker ?: "(no-marker)"
//
// therefore yields the default when the CALL ITSELF throws. RustCFML used to
// tolerate only undefined reads (tolerant member ops), so a genuine throw on
// the left propagated and killed the request — measured divergence on legs
// 5/6/10/12 below, all `DEF` on Lucee 7.0.4 and all THREW here before the fix.
//
// Found by @Blute while writing the getBaseTagData() instance-number suite
// (PR #323): his first probe chained the call through `?:` and the two engines
// appeared to disagree about getBaseTagData() itself. They did not — the elvis
// was swallowing the throw on one engine only.
//
// The guard covers the LEFT OPERAND ONLY: leg 7 is the control that pins that
// the same throw WITHOUT `?:` still propagates on both engines.
//
// NOT pinned here (measured, deliberately not adopted): Lucee additionally
// restricts the elvis LHS grammatically at COMPILE time — "left operand of the
// Elvis operator has to be a variable or a function call" — so `(1/0) ?: "d"`
// is a compile error there and merely evaluates here. Nothing in this file may
// use that shape, or Lucee fails to compile the suite at all.

suiteBegin("Elvis ?: absorbs exceptions from its left operand (Lucee parity, GH ##329)");

// Runs fn() and reports either the value or the escaping exception, so a leg
// that throws is asserted as a VALUE instead of aborting the suite.
function elvisProbe(required any fn) {
	try { return "[" & arguments.fn() & "]"; }
	catch (any e) { return "THREW " & e.type & ": " & e.message; }
}

function elvisBoom() {
	throw(type="ElvisProbeCustom", message="boom went the fn");
}

function elvisGivesNull() {
	return javacast("null", "");
}

elvisStruct = { a: 1 };
elvisArr = [ 1 ];

// ── Already-correct legs: undefined reads on the left ──
assert( "1 undefined variable yields the default",
	elvisProbe( function() { return elvisNoSuchVar ?: "DEF"; } ), "[DEF]" );
assert( "2 undefined key on a defined struct yields the default",
	elvisProbe( function() { return elvisStruct.missing ?: "DEF"; } ), "[DEF]" );
assert( "3 deep chain through an undefined intermediate yields the default",
	elvisProbe( function() { return elvisStruct.missing.deeper ?: "DEF"; } ), "[DEF]" );
assert( "9 member access on a null-returning function yields the default",
	elvisProbe( function() { return elvisGivesNull().marker ?: "DEF"; } ), "[DEF]" );
assert( "11 array index out of range yields the default",
	elvisProbe( function() { return elvisArr[ 5 ] ?: "DEF"; } ), "[DEF]" );

// ── A throwing BIF on the left ──
assert( "4 a throwing BIF on the left yields the default",
	elvisProbe( function() { return structFind( elvisStruct, "nope" ) ?: "DEF"; } ), "[DEF]" );

// ── THE GAP: real exceptions raised on the left ──
assert( "5 a UDF that throws yields the default",
	elvisProbe( function() { return elvisBoom() ?: "DEF"; } ), "[DEF]" );
assert( "6 member access on a throwing call yields the default",
	elvisProbe( function() { return elvisBoom().attributes.marker ?: "DEF"; } ), "[DEF]" );
assert( "10 the guard holds inside a larger expression",
	elvisProbe( function() { return "x" & ( elvisBoom() ?: "DEF" ); } ), "[xDEF]" );
assert( "12 a throw nested in an argument is still absorbed",
	elvisProbe( function() { return len( elvisBoom() ) ?: "DEF"; } ), "[DEF]" );

// ── Control: the guard covers the LEFT OPERAND ONLY ──
// Without `?:` the identical throw must still propagate, or the fix has turned
// `?:` into a blanket error suppressor for the surrounding expression.
assert( "7 control: the same throw WITHOUT ?: still propagates",
	elvisProbe( function() { return elvisBoom(); } ),
	"THREW ElvisProbeCustom: boom went the fn" );

// ── Control: a non-null left operand is returned untouched ──
assert( "13 control: a present value on the left wins over the default",
	elvisProbe( function() { return elvisStruct.a ?: "DEF"; } ), "[1]" );
assert( "14 control: a successful call on the left wins over the default",
	elvisProbe( function() { return len( "abcd" ) ?: "DEF"; } ), "[4]" );

// ── Control: the default is only evaluated when it is actually needed ──
// A throwing DEFAULT must not fire when the left operand is present.
assert( "15 control: the default is not evaluated when the left is present",
	elvisProbe( function() { return elvisStruct.a ?: elvisBoom(); } ), "[1]" );

suiteEnd();
</cfscript>
