<cfscript>
// An include runs in its caller's scopes: Lucee runs it on the same
// PageContext, so the template reads AND writes the calling function's
// `local`, `arguments` and closures exactly as the function body would.
// Every expected value below is Lucee 7.1's (fixtures in frame_model/).
suiteBegin( "An include runs in the calling function's scopes" );

// P1: an existing local, an argument, and a new name
function fmP1( fmArgP = "arg" ) {
	var fmExisting = "orig";
	include "frame_model/p1_write.cfm";
	return {
		existing: fmExisting,
		arg: fmArgP,
		argScoped: arguments.fmArgP,
		newInLocal: structKeyExists( local, "fmNewName" ),
		newInVariables: structKeyExists( variables, "fmNewName" )
	};
}
r = fmP1();
assert( "an include updates an existing local", r.existing, "changed" );
assert( "an include updates an argument (bare read)", r.arg, "argchanged" );
assert( "an include updates an argument (arguments.x)", r.argScoped, "argchanged" );
assertFalse( "a new unscoped name does not become a local", r.newInLocal );
assertTrue( "a new unscoped name lands in variables (classic)", r.newInVariables );
structDelete( variables, "fmNewName" );

// P3: reassigning a caller local alongside a builtin-named and a function-named key
function fmPageFn() { return "fn"; }
function fmP3() {
	var fmCallerVar = "orig";
	include "frame_model/p3_reassign.cfm";
	return { callerVar: fmCallerVar, valInLocal: structKeyExists( local, "val" ), valInVariables: structKeyExists( variables, "val" ) };
}
r = fmP3();
assert( "an include reassigns a caller local", r.callerVar, "fromInclude" );
assertFalse( "a builtin-named write is not a local", r.valInLocal );
assertTrue( "a builtin-named write lands in variables", r.valInVariables );
structDelete( variables, "val" );

// P4: localmode="modern" reaches into the include
function fmP4() localmode="modern" {
	include "frame_model/p4_modern.cfm";
	return { inLocal: structKeyExists( local, "fmModernNew" ), inVariables: structKeyExists( variables, "fmModernNew" ) };
}
r = fmP4();
assertTrue( "modern localmode: a new name in an include is local", r.inLocal );
assertFalse( "modern localmode: a new name in an include is not in variables", r.inVariables );

// P6: a closure defined in an include sees the including function's scopes, live
o = new frame_model.FrameModelProbe();
r = o.closureFromInclude();
assertTrue( "closure from an include in a method sees variables", r.sawVariables );
assertTrue( "closure from an include in a method sees this", r.sawThis );
assertTrue( "closure from an include in a method sees the method's locals", r.sawMethodLocal );
assert( "closure from an include in a method reads the local's CURRENT value", r.methodLocalValue, "after" );

// P7: `local` and `arguments` as values across an include
function fmP7( a = 1 ) {
	var localRef = local;
	var argsRef = arguments;
	include "frame_model/p7_writes.cfm";
	return {
		localRefSeesLocalAdded: structKeyExists( localRef, "fmLocalAdded" ),
		argsRefSeesArgAdded: structKeyExists( argsRef, "fmArgAdded" ),
		argsSeesArgAdded: structKeyExists( arguments, "fmArgAdded" ),
		viaIncludeInVariables: structKeyExists( variables, "fmViaInclude" )
	};
}
r = fmP7();
assertTrue( "local.x written in an include is the caller's local", r.localRefSeesLocalAdded );
assertTrue( "arguments.x written in an include is the caller's arguments (held reference)", r.argsRefSeesArgAdded );
assertTrue( "arguments.x written in an include is the caller's arguments", r.argsSeesArgAdded );
assertTrue( "a new unscoped name in an include lands in variables", r.viaIncludeInVariables );
structDelete( variables, "fmViaInclude" );

// P8: three nested includes inside a function
function fmP8() {
	var fmL1 = "orig";
	include "frame_model/p8_level1.cfm";
	return { l1: fmL1, l2InVariables: structKeyExists( variables, "fmL2" ), l3InVariables: structKeyExists( variables, "fmL3" ) };
}
r = fmP8();
assert( "a write three includes deep updates the function's local", r.l1, "one-by-3" );
assertTrue( "level-2 new name lands in variables", r.l2InVariables );
assertTrue( "level-3 new name lands in variables", r.l3InVariables );
structDelete( variables, "fmL2" );
structDelete( variables, "fmL3" );

// P10: an include that throws keeps the writes it made before the throw
function fmP10() {
	var fmThrowLocal = "orig";
	try {
		include "frame_model/p10_throw.cfm";
	} catch ( fmProbe e ) {
	}
	return fmThrowLocal;
}
assert( "an include that throws keeps its earlier writes", fmP10(), "changed" );

// P11: a closure the include assigns to a caller local replaces it
function fmP11() {
	var fmCb = function() { return "fromCaller"; };
	include "frame_model/p11_closure.cfm";
	return fmCb();
}
assert( "a closure assigned in an include replaces the caller's", fmP11(), "fromInclude" );

// P12: an include with its own loop, inside the caller's loop
function fmP12() {
	var fmLoopTrace = "";
	for ( var fmOuter in [ 1, 2, 3 ] ) {
		fmLoopTrace &= fmOuter;
		include "frame_model/p12_loop.cfm";
	}
	return fmLoopTrace;
}
assert( "a loop in an include leaves the caller's loop intact", fmP12(), "1ab2ab3ab" );
structDelete( variables, "fmInner" );

// P12 at page level
fmLoopTrace = "";
for ( fmPageOuter in [ 1, 2 ] ) {
	fmLoopTrace &= fmPageOuter;
	include "frame_model/p12_loop.cfm";
}
assert( "a loop in a page-level include leaves the page's loop intact", fmLoopTrace, "1ab2ab" );
structDelete( variables, "fmInner" );
structDelete( variables, "fmLoopTrace" );
structDelete( variables, "fmPageOuter" );

suiteEnd();
</cfscript>
