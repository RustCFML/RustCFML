<cfscript>
// The metadata executed-template cache (request-scoped) lets getComponentMetaData
// reuse one execution of a component body across a request. These asserts pin the
// two things that cache could plausibly break:
//
//  1. Metadata stays correct and independent when several classes share a parent
//     (the shared parent is exactly what the cache serves repeatedly), and when
//     the SAME class is asked for twice — a caller mutating the struct it is
//     handed must not poison the next caller.
//  2. createObject / new are NEVER served from it. CFML runs a pseudo-constructor
//     once per instantiation; the cache is scoped to metadata derivation only, and
//     that boundary is the whole safety argument. A regression here is SILENT —
//     instances would quietly share construction-time state — so it is asserted
//     directly rather than trusted.

suiteBegin("Component metadata template cache");

// --- 1. shared parent, distinct children -----------------------------------
mdA = getComponentMetaData("oop.metacache.ChildA");
mdB = getComponentMetaData("oop.metacache.ChildB");

assert("ChildA name", listLast(mdA.name, "."), "ChildA");
assert("ChildB name", listLast(mdB.name, "."), "ChildB");
assert("ChildA parent", listLast(mdA.extends.name, "."), "SharedBase");
assert("ChildB parent", listLast(mdB.extends.name, "."), "SharedBase");

// Each child's own function is present and NOT leaked onto its sibling.
function hasFn(md, name) {
	for (f in md.functions) {
		if (f.name == name) { return true; }
	}
	return false;
}
assert("ChildA has ownA", hasFn(mdA, "ownA"), true);
assert("ChildB has ownB", hasFn(mdB, "ownB"), true);
assertFalse("ChildA lacks ownB", hasFn(mdA, "ownB"));
assertFalse("ChildB lacks ownA", hasFn(mdB, "ownA"));

// --- 2. a mutating caller must not poison the memo -------------------------
// ColdBox's getInheritedMetaData edits the struct it is given; entries are
// stored and returned as deep copies precisely so this cannot propagate.
mdA.name = "MUTATED";
mdA.extends.name = "MUTATED_PARENT";
mdA2 = getComponentMetaData("oop.metacache.ChildA");
assert("re-read name unpoisoned", listLast(mdA2.name, "."), "ChildA");
assert("re-read parent unpoisoned", listLast(mdA2.extends.name, "."), "SharedBase");

// --- 3. THE INVARIANT: instantiation still runs the pseudo-constructor ------
// SharedBase increments a request-scoped counter in its body. Reading metadata
// must not be able to satisfy an instantiation, so two `new` calls must produce
// two distinct construction stamps.
request.metacacheCtorRuns = 0;
o1 = new oop.metacache.ChildA();
firstRuns = request.metacacheCtorRuns;
o2 = new oop.metacache.ChildA();
secondRuns = request.metacacheCtorRuns;

assertTrue("first instantiation ran the body", firstRuns >= 1);
assertTrue("second instantiation ran the body again", secondRuns > firstRuns);
assertFalse("instances do not share a construction stamp", o1.getStamp() == o2.getStamp());

// Interleaving metadata reads with instantiation must not change that.
md3 = getComponentMetaData("oop.metacache.ChildA");
before = request.metacacheCtorRuns;
o3 = new oop.metacache.ChildA();
assertTrue("instantiation after a metadata read still runs the body",
	request.metacacheCtorRuns > before);
assertFalse("third instance distinct too", o3.getStamp() == o1.getStamp());

suiteEnd();
</cfscript>
