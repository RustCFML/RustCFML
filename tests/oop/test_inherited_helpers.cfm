<cfscript>
suiteBegin("Inherited helpers visible during child cfc body (Bug G)");

c = new oop.InheritedHelpersChild();

// Child's pseudo-constructor referenced parent's variables.encode.string;
// before the fix, that threw silently, leaving the member scope empty.
//
// Asserted through the component's PUBLIC accessor, not `c.__variables`.
// GH #417: a component's private member scope is not externally readable —
// Lucee answers `c.__variables` with "has no accessible Member with name
// [__VARIABLES]", so reaching in was pinning a RustCFML-only leak. `get()`
// returns `variables.dummyData`, so it observes exactly the same thing the
// direct reads did.
got = c.get();
assert("dummyData survives", isStruct(got) && !structIsEmpty(got), true);
assert("dummyData.whatever set", got.whatever, true);
assert("dummyData.encoded carries function result", got.encoded, "ENC:payload");

// Method that reads through variables.dummyData also returns the populated struct.
assert("get() returns whatever", got.whatever, true);
assert("get() returns encoded", got.encoded, "ENC:payload");

// Multiple instances should be independent and reproducible.
c2 = new oop.InheritedHelpersChild();
assert("second instance encoded", c2.get().encoded, "ENC:payload");

suiteEnd();
</cfscript>
