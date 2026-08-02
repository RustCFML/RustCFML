<cfscript>
// GH #307 — silent-no-op member functions.
//
// `.add()` on an array appended nothing and threw nothing, so calling code took the
// success path with an empty array (Preside TaskManagerService.listTasks() returned
// [] for every call). The root cause was not one missing arm but the terminal
// `Ok(CfmlValue::Null)` in call_member_function: EVERY unhandled member call on a
// typed receiver was a silent null. This suite pins both the newly-wired members and
// the throw-on-unknown behaviour that stops the class recurring.
//
// Every expectation below was verified against Lucee 7.0.4.34 via CommandBox.
suiteBegin("Member function coverage (GH ##307)");

// ---- java.util.List passthroughs on arrays -------------------------------
// Java methods: ZERO-based indices, case-sensitive comparisons.
a = [1];
assertTrue("add() returns true", a.add(2));
assert("add() appends", arrayToList(a), "1,2");

// The exact shape from the issue: filter-into-a-new-array via .add().
filtered = [];
for (item in ["keep", "drop", "keep"]) {
    if (item == "keep") { filtered.add(item); }
}
assert("add() in a loop collects matches", arrayToList(filtered), "keep,keep");

a = [1, 3];
a.add(1, 2);
assert("add(index, elem) inserts 0-based", arrayToList(a), "1,2,3");

assert("get() is 0-based", [9, 8].get(1), 8);
assertThrows("get() out of range throws", function() { return [1].get(5); });

a = ["x", "y"];
assertTrue("remove(value) reports removal", a.remove("x"));
assert("remove(value) removes by VALUE", arrayToList(a), "y");
a = [10, 20, 30];
assertFalse("remove() is remove(Object), not remove(int)", a.remove(1));
assert("remove() miss leaves array intact", arrayToList(a), "10,20,30");

a = [1, 2, 3];
assertTrue("removeAll() reports change", a.removeAll([2]));
assert("removeAll() drops listed values", arrayToList(a), "1,3");

a = [1, 2, 3];
a.retainAll([2]);
assert("retainAll() keeps only listed values", arrayToList(a), "2");

assert("subList() is 0-based, end-exclusive", arrayToList([1,2,3,4].subList(1, 3)), "2,3");
assertTrue("containsAll() true when all present", [1,2,3].containsAll([1,2]));
assertFalse("containsAll() false when one absent", [1,2,3].containsAll([1,9]));

// indexOf is java.util.List.indexOf — 0-based, -1 when absent. It is NOT arrayFind
// (1-based, 0 when absent); the old alias made `if (x.indexOf(v) >= 0)` always true.
assert("array indexOf() is 0-based", ["a","b","c"].indexOf("b"), 1);
assert("array indexOf() miss is -1", ["a"].indexOf("z"), -1);
assert("array lastIndexOf() is 0-based", [1,2,1].lastIndexOf(1), 2);
assert("arrayFind stays 1-based", ["a","b","c"].find("b"), 2);

// ---- CFML array members whose BIFs existed but were never wired up -------
a = [1, 2];
assert("pop() returns the last element", a.pop(), 2);
assert("pop() shortens the array", arrayLen(a), 1);
a = [1, 2];
assert("shift() returns the first element", a.shift(), 1);
assert("shift() shortens the array", arrayToList(a), "2");
a = [2];
a.unshift(1);
assert("unshift() prepends", arrayToList(a), "1,2");
a = [1, 2];
a.swap(1, 2);
assert("swap() exchanges elements", arrayToList(a), "2,1");
a = [1];
a.resize(3);
assert("resize() grows the array", arrayLen(a), 3);
assertTrue("resize() fills with null, not empty string", isNull(a[2]));
assert("median()", [1,2,3].median(), 2);
assert("removeDuplicates()", arrayToList([1,1,2].removeDuplicates()), "1,2");
assert("mid()", arrayToList([1,2,3,4].mid(2, 2)), "2,3");
a = [1, 2, 3];
assert("splice() returns the removed slice", arrayToList(a.splice(2, 1)), "2");
assert("splice() mutates the receiver", arrayToList(a), "1,3");
assertTrue("indexExists()", [1,2].indexExists(2));
s = [1,2].toStruct();
assert("toStruct()", s["1"], 1);
orig = [1];
copy = orig.duplicate();
copy.append(2);
assert("duplicate() is a deep copy", arrayLen(orig), 1);

// arraySet used to return `false` and mutate nothing when under-supplied — a
// silent no-op. Exercised through the member form: a bare `arraySet([1,2])` is a
// COMPILE error on Lucee (static arity check), which would abort this whole file
// there rather than testing anything.
assertThrows("under-supplied set() throws rather than no-op", function() {
    a = [1, 2];
    return a.set(1, "z");
});

// ---- java.util.Map passthroughs on structs -------------------------------
// The receiver must SURVIVE: `put` is in is_mutating_method, so returning a value
// rather than the receiver once clobbered the struct variable with that value.
st = { a: 1 };
assert("put() returns the stored value", st.put("k", "v"), "v");
assert("put() does not clobber the struct", st.k, "v");
assert("put() keeps existing keys", st.a, 1);
st = { k: "old" };
assert("putIfAbsent() returns the retained value", st.putIfAbsent("k", "new"), "old");
assert("putIfAbsent() does not overwrite", st.k, "old");
assert("remove() returns the removed value", st.remove("k"), "old");
assertFalse("remove() deletes the key", structKeyExists(st, "k"));
assertTrue("containsKey()", { a: 1 }.containsKey("a"));
assertFalse("containsKey() miss", { a: 1 }.containsKey("zz"));
assertTrue("containsValue()", { a: 1 }.containsValue(1));
// keySet()/values()/entrySet() are asserted through ITERATION rather than by
// casting to an array: Lucee returns live java.util.Collection views (and its
// Values view cannot be cast or looped at all), whereas RustCFML has no Java
// collection type and hands back a CFML array. Iteration is the shape both
// engines support, and it is what real callers do. See docs/known-issues.md.
seenKeys = [];
for (k in { a: 1, b: 2 }.keySet()) { seenKeys.append(lCase(k)); }
assert("keySet() iterates every key", arrayToList(seenKeys.sort("text")), "a,b");
entryCount = 0;
for (e in { a: 1 }.entrySet()) { entryCount++; }
assert("entrySet() iterates every entry", entryCount, 1);
assertFalse("values() returns a usable collection", isNull({ a: 1 }.values()));

// ---- java.lang.String passthroughs ---------------------------------------
assert("charAt() is 0-based", "ab".charAt(0), "a");
assert("substring(begin, end) is 0-based/exclusive", "abcd".substring(1, 3), "bc");
assert("substring(begin) runs to the end", "abcd".substring(2), "cd");
assert("concat()", "a".concat("b"), "ab");
assertTrue("equalsIgnoreCase()", "AB".equalsIgnoreCase("ab"));
assertFalse("equals() is case-sensitive", "AB".equals("ab"));
assert("string indexOf() is 0-based", "abc".indexOf("b"), 1);
assert("string indexOf() miss is -1", "abc".indexOf("z"), -1);
assert("string lastIndexOf() is 0-based", "abcb".lastIndexOf("b"), 3);
assert("replaceAll() takes a regex", "aXb".replaceAll("X", "-"), "a-b");
assert("compareTo() sign", "a".compareTo("b"), -1);
// java.lang.String.hashCode — must match Java exactly so a value hashed on Lucee
// and on RustCFML lands in the same bucket.
assert("hashCode() matches Java", "ab".hashCode(), 3105);
assertTrue("isBlank()", "   ".isBlank());

// ---- The systemic fix: unknown members THROW, never return null ----------
// This is what turns the next gap in the tables above into a loud bug report
// instead of silent data loss.
assertThrows("unknown array member throws", function() { return [1].noSuchMember(); });
assertThrows("unknown string member throws", function() { return "a".noSuchMember(); });
assertThrows("unknown query member throws", function() { return queryNew("a").noSuchMember(); });
assertThrows("unknown numeric member throws", function() { n = 5; return n.noSuchMember(); });
assertThrows("unknown boolean member throws", function() { b = true; return b.noSuchMember(); });

suiteEnd();
</cfscript>
