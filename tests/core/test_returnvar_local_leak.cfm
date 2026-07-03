<cfscript>
suiteBegin("cfinvoke returnVariable='local.rv' stays frame-private (no caller leak)");

// A callee's `local.rv` (populated via cfinvoke returnVariable) must not leak
// into the caller's `local` scope. Before the fix, wrapInvoke's local.rv=true
// leaked into outer(), clobbering outer's own local vars — Wheels nested-save
// PK rollback regression (nestedpropertiesSpec).
rvl = new ReturnVarLocalLeakFixture();
rvlOut = rvl.outer();

// outerRv must stay "OUTER" for all 3 iterations (never overwritten by the
// callee's local.rv).
assertFalse("caller local var not clobbered by callee cfinvoke returnVariable",
	rvlOut contains "outerRv=true");
assertTrue("caller local var keeps its own value across the loop",
	(rvlOut contains "outerRv=OUTER"));
// The callee still returns its own captured result correctly.
assertTrue("callee cfinvoke returnVariable still returns the value", rvlOut contains "cr=true");
// The parent's running accumulator set false at i=2 must NOT be reverted to true
// at i=3 by the callee's successful invoke (the exact $saveAssociations shape).
assertTrue("parent running accumulator not reverted after callee invoke",
	rvlOut contains "i=3:outerRv=OUTER,accum=false");

suiteEnd();
</cfscript>
