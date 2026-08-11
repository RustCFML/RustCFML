<cfscript>
// Issue #187: an overflow named argument (a named arg with NO matching declared
// param) must NOT leak into the CALLER's local scope. Wheels models call
// `validatesLengthOf(property = "x")` — whose declared param is `properties`
// (plural) — so `property` is an overflow arg. The legacy-localmode parent-scope
// writeback was treating that arg as a fresh local write and copying it back into
// the calling method's locals, where it shadowed the in-scope `property()`
// function on the next bare call (a string "x" is not callable -> 500).
suiteBegin("Overflow named arg no caller leak (issue 187)");

obj = new OverflowArgProbe();
assertTrue("bare fn after overflow-arg call still resolves to the function", obj.runConfig() eq "PROP:salesTotal");

// The exclusion is case-insensitive: an overflow arg named `PROPERTY` must be
// recognised as an arguments-scope key just as `property` is, or it leaks into
// the caller's locals and shadows the property() function on the next bare call.
assertTrue("case-mismatched overflow named arg also does not leak", obj.runConfigMixedCase() eq "PROP:mixedCase");

suiteEnd();
</cfscript>
