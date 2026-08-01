<cfscript>
// Component-path resolution cache (GH #298). The two cache layers are keyed by
// (class name case-insensitively, calling template's DIRECTORY, base template),
// hashed and then verified — so these are the cases the key must keep apart and
// the ones it is allowed to share.
suiteBegin("Component resolution cache");

// Same bare class name, two directories: each caller must get the Shared.cfc
// next to itself. If the caller part of the key were dropped, the second lookup
// would hit the first one's entry and return the wrong component.
a = new oop.rescache_a.Caller();
b = new oop.rescache_b.Caller();
assert("bare name resolves next to caller (dir a)", a.resolve(), "a");
assert("bare name resolves next to caller (dir b)", b.resolve(), "b");
// Repeat, reversed — now both entries are warm, so a mis-keyed hit would show up.
assert("dir b still resolves to b when warm", b.resolve(), "b");
assert("dir a still resolves to a when warm", a.resolve(), "a");

// Two DIFFERENT caller files in the SAME directory share one cache entry (the
// key is the caller's dir, not the file). Both must still resolve correctly.
a2 = new oop.rescache_a.Caller2();
assert("sibling caller in the same dir shares the entry", a2.resolve(), "a");
assert("first caller unaffected by the shared entry", a.resolve(), "a");

// Class names are case-insensitive, and differ only by case here: the same
// entry must serve both spellings.
assert("dotted path resolves", new oop.rescache_a.Shared().whoAmI(), "a");
assert("dotted path, mixed case", new OOP.RESCACHE_A.shared().whoAmI(), "a");

// A path that does not resolve stays unresolved (negative results are never
// cached as successes, and the error is the component-not-found one).
assertThrows("missing component still throws", function() {
	new oop.rescache_a.NoSuchComponentHere();
});
// ...and a real one still resolves afterwards.
assert("resolution still works after a miss", a.resolve(), "a");

suiteEnd();
</cfscript>
