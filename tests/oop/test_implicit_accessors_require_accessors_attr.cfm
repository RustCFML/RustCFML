<cfscript>
// GH #421. Implicit getX/setX exist ONLY for a property DECLARED on a component
// with accessors="true". Everything else is "has no function with name [X]".
//
// We used to synthesize an accessor for any component that had no
// onMissingMethod, so `setAnything("x")` always succeeded and created a
// variable — a typo'd or entirely invented setter wrote silently instead of
// raising. All six expectations below were measured on Lucee 7.1.0.204.
//
// The onMissingMethod route is deliberately NOT covered here: it has its own
// regression test (test_property_no_accessors_onmissing.cfm) and is unchanged.
suiteBegin("Implicit accessors require accessors=true");

off = createObject("component", "AccessorsOffBean");

// --- accessors OFF: nothing is synthesized, declared or not ---------------
assertThrows("declared setter throws without accessors", function() {
	off.setUsername("testuser");
});
assertThrows("undeclared setter throws without accessors", function() {
	off.setRandomVar("Foo");
});
assertThrows("declared setter via invoke() throws without accessors", function() {
	invoke(off, "setEmail", {"email"="a@b.c"});
});
assertThrows("undeclared setter via invoke() throws without accessors", function() {
	invoke(off, "setLogin", {"login"="x"});
});

// A silent write is the failure this pins: before the fix the calls above
// returned cleanly and left the value behind.
assertFalse("no variable was created by the refused setter",
	structKeyExists(off, "randomVar"));

// --- accessors ON: declared properties work, undeclared still do not ------
on = createObject("component", "AccessorsOnBean");

on.setUsername("testuser");
assert("declared accessor round-trips with accessors=true",
	on.getUsername(), "testuser");

assertThrows("undeclared setter throws even with accessors=true", function() {
	on.setRandomVar("Foo");
});
assertThrows("undeclared getter throws even with accessors=true", function() {
	on.getRandomVar();
});

suiteEnd();
</cfscript>
