<cfscript>
suiteBegin("A method-body include declares instance methods (GH 481 follow-up, ColdBox includeUDF mixins)");

// Probed on Lucee 7.1: a named function declared by a template included from a
// component METHOD becomes a public method of that INSTANCE. It does not see
// the including method's locals at call time, and it is not callable as a bare
// page function afterwards.
mh = new oop.fixtures.gh481.MixinHost().init();
mh.loadHelpers();
assert("the helper is a public member of this", structKeyExists( mh, "probeLocal" ), true);
assert("the helper does NOT see the including method's locals",
	mh.probeLocal(), "NO-METHOD-LOCAL");
assert("the helper reads the instance's variables scope",
	mh.probeVars() != "NO-INSTANCE-VAR", true);
bareWorks = true;
try { probeLocal(); } catch ( any e ) { bareWorks = false; }
assert("the helper is NOT a bare page function", bareWorks, false);

mh2 = new oop.fixtures.gh481.MixinHost().init();
mh2.loadHelpers();
mh.probeWrite(); mh.probeWrite();
assert("helper state is per instance", mh2.probeWrite(), 1);
assert("each instance reads its own variables", mh2.probeVars() != mh.probeVars(), true);

suiteEnd();
</cfscript>
