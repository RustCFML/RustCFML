// Lucee 7 Compatibility Tests: Struct Functions
// Synthesized from https://github.com/lucee/Lucee/tree/7.0/test/functions
// Original tests Copyright (c) 2014, the Railo Company LLC / Copyright (c) 2015-2016, Lucee Association Switzerland
// Licensed under the GNU Lesser General Public License v2.1
// Adapted for RustCFML test harness
<cfscript>
suiteBegin("Lucee7: Struct Functions");

// ============================================================
// StructNew (from Lucee structNew.cfc)
// ============================================================
sn = structNew();
assertTrue("structNew returns struct", isStruct(sn));
assert("structNew is empty", structCount(sn), 0);
assertTrue("structNew isEmpty", structIsEmpty(sn));

// ============================================================
// StructCount / StructIsEmpty (from Lucee structCount.cfc, structisempty.cfc)
// ============================================================
sc = {a:1, b:2};
assert("structCount 2 keys", structCount(sc), 2);
assertTrue("structIsEmpty on empty struct", structIsEmpty({}));
assertFalse("structIsEmpty on non-empty struct", structIsEmpty({a:1}));
sc2 = {a:1, b:2, c:3, d:4};
assert("structCount 4 keys", structCount(sc2), 4);

// ============================================================
// isStruct (from Lucee IsStruct.cfc)
// ============================================================
assertTrue("isStruct on struct literal", isStruct({}));
assertTrue("isStruct on populated struct", isStruct({a:1}));
assertFalse("isStruct on array", isStruct([]));
assertFalse("isStruct on string", isStruct("hello"));
assertFalse("isStruct on number", isStruct(42));
assertFalse("isStruct on boolean", isStruct(true));

// ============================================================
// StructKeyExists (from Lucee structkeyexist.cfc)
// ============================================================
ske = {name:"test", age:30};
assertTrue("structKeyExists existing key", structKeyExists(ske, "name"));
assertTrue("structKeyExists case insensitive", structKeyExists(ske, "NAME"));
assertFalse("structKeyExists missing key", structKeyExists(ske, "missing"));
assertFalse("structKeyExists empty string key", structKeyExists(ske, ""));

// ============================================================
// StructKeyList / StructKeyArray (from Lucee structkeylist.cfc, structkeyarray.cfc)
// ============================================================
skl = {a:1, b:2, c:3};
assert("structKeyList length", listLen(structKeyList(skl)), 3);
assert("structKeyList custom delim length", listLen(structKeyList(skl, "|"), "|"), 3);
ska = structKeyArray(skl);
assert("structKeyArray length", arrayLen(ska), 3);
// empty struct
assert("structKeyList empty", structKeyList({}), "");
assert("structKeyArray empty", arrayLen(structKeyArray({})), 0);

// ============================================================
// StructDelete (from Lucee structdelete.cfc)
// ============================================================
sd = {a:1, b:2, c:3};
structDelete(sd, "a");
assertFalse("structDelete removes key", structKeyExists(sd, "a"));
assert("structDelete count after", structCount(sd), 2);
// delete non-existing key should not error
structDelete(sd, "nonexistent");
assert("structDelete non-existing no error", structCount(sd), 2);

// ============================================================
// StructInsert (from Lucee structinsert.cfc)
// ============================================================
si = {};
structInsert(si, "key", "value");
assert("structInsert adds key", si.key, "value");
assert("structInsert count", structCount(si), 1);
structInsert(si, "another", 42);
assert("structInsert second key", si.another, 42);
assert("structInsert count after second", structCount(si), 2);

// ============================================================
// StructUpdate (from Lucee StructUpdate.cfc)
// ============================================================
su = {a:1, b:2};
structUpdate(su, "a", 10);
assert("structUpdate changes value", su.a, 10);
structUpdate(su, "b", "hello");
assert("structUpdate changes type", su.b, "hello");

// ============================================================
// StructFind (from Lucee structFind.cfc)
// ============================================================
sf = {name:"test", count:5};
assert("structFind by key", structFind(sf, "name"), "test");
assert("structFind numeric value", structFind(sf, "count"), 5);
assert("structFind case insensitive", structFind(sf, "NAME"), "test");

// ============================================================
// StructFindKey (from Lucee structfindkey.cfc)
// ============================================================
sfk = {a:{b:1}, c:2};
sfkResult = structFindKey(sfk, "b");
assert("structFindKey finds nested key", arrayLen(sfkResult), 1);
assert("structFindKey result value", sfkResult[1].value, 1);
// top-level key
sfkResult2 = structFindKey(sfk, "c");
assert("structFindKey finds top-level key", arrayLen(sfkResult2), 1);
assert("structFindKey top-level value", sfkResult2[1].value, 2);
// non-existing key
sfkResult3 = structFindKey(sfk, "z");
assert("structFindKey missing key", arrayLen(sfkResult3), 0);
// deeper nesting
sfkDeep = {level1:{level2:{target:"found"}}};
sfkDeepResult = structFindKey(sfkDeep, "target");
assert("structFindKey deep nesting", arrayLen(sfkDeepResult), 1);
assert("structFindKey deep value", sfkDeepResult[1].value, "found");

// ============================================================
// StructFindValue (from Lucee structFindValue.cfc)
// ============================================================
sfv = {a:"hello", b:"world", c:"hello"};
sfvResult = structFindValue(sfv, "hello");
assert("structFindValue finds value", arrayLen(sfvResult), 1);
sfvResult2 = structFindValue(sfv, "notfound");
assert("structFindValue missing value", arrayLen(sfvResult2), 0);
// nested struct - default scope searches all levels in RustCFML
sfvNested = {x:{y:"deep"}};
sfvNestedAll = structFindValue(sfvNested, "deep", "all");
assert("structFindValue all finds nested", arrayLen(sfvNestedAll), 1);

// ============================================================
// StructClear (from Lucee structclear.cfc)
// ============================================================
scl = {a:1, b:2, c:3};
assert("structClear before count", structCount(scl), 3);
structClear(scl);
assertTrue("structClear empties struct", structIsEmpty(scl));
assert("structClear count is 0", structCount(scl), 0);

// ============================================================
// StructCopy (from Lucee StructCopy.cfc)
// ============================================================
scp1 = {x:"one", y:"two"};
scp2 = structCopy(scp1);
assert("structCopy preserves values x", scp2.x, "one");
assert("structCopy preserves values y", scp2.y, "two");
// shallow copy: modifying copy does not affect original
scp2.x = "modified";
assert("structCopy shallow - original unchanged", scp1.x, "one");
assert("structCopy shallow - copy changed", scp2.x, "modified");
// adding to copy does not affect original
scp2.z = "new";
assertFalse("structCopy - new key not in original", structKeyExists(scp1, "z"));

// ============================================================
// StructAppend (from Lucee structAppend.cfc)
// ============================================================
sa1 = {a:1};
sa2 = {b:2, c:3};
structAppend(sa1, sa2);
assertTrue("structAppend adds key b", structKeyExists(sa1, "b"));
assertTrue("structAppend adds key c", structKeyExists(sa1, "c"));
assert("structAppend count", structCount(sa1), 3);
assert("structAppend value b", sa1.b, 2);
// overwrite behavior
sa3 = {a:1};
sa4 = {a:99};
structAppend(sa3, sa4);
assert("structAppend overwrites", sa3.a, 99);
// no overwrite
sa5 = {a:1};
sa6 = {a:99, b:2};
structAppend(sa5, sa6, false);
assert("structAppend no overwrite keeps original", sa5.a, 1);
assertTrue("structAppend no overwrite adds new", structKeyExists(sa5, "b"));

// ============================================================
// StructSort (from Lucee structsort.cfc)
// ============================================================
ss = {c:3, a:1, b:2};
sorted = structSort(ss, "numeric");
assert("structSort numeric first key", sorted[1], "A");
assert("structSort numeric last key", sorted[3], "C");
// text sort
sst = {b:"banana", a:"apple", c:"cherry"};
sortedText = structSort(sst, "text");
assert("structSort text first", sortedText[1], "A");
assert("structSort text second", sortedText[2], "B");
assert("structSort text third", sortedText[3], "C");

// ============================================================
// StructEach (from Lucee structEach.cfc)
// ============================================================
se = {a:1, b:2, c:3};
seKeys = "";
seTotal = 0;
structEach(se, function(k, v) {
    seKeys = listAppend(seKeys, k);
    seTotal = seTotal + v;
});
assert("structEach visits all keys", listLen(seKeys), 3);
assert("structEach sum of values", seTotal, 6);

// ============================================================
// StructMap (from Lucee structMap.cfc)
// ============================================================
sm = {a:1, b:2, c:3};
smResult = structMap(sm, function(k, v) {
    return v * 2;
});
assert("structMap a doubled", smResult.a, 2);
assert("structMap b doubled", smResult.b, 4);
assert("structMap c doubled", smResult.c, 6);
assert("structMap count", structCount(smResult), 3);
// original unchanged
assert("structMap original unchanged", sm.a, 1);

// ============================================================
// StructFilter (from Lucee structFilter.cfc)
// ============================================================
sfl = {a:1, b:2, c:3, d:4};
sflResult = structFilter(sfl, function(k, v) {
    return v > 2;
});
assert("structFilter count", structCount(sflResult), 2);
assertTrue("structFilter has c", structKeyExists(sflResult, "c"));
assertTrue("structFilter has d", structKeyExists(sflResult, "d"));
assertFalse("structFilter excludes a", structKeyExists(sflResult, "a"));
assertFalse("structFilter excludes b", structKeyExists(sflResult, "b"));

// ============================================================
// StructReduce (from Lucee structReduce.cfc)
// ============================================================
sr = {a:1, b:2, c:3};
srTotal = structReduce(sr, function(acc, k, v) {
    return acc + v;
}, 0);
assert("structReduce sum", srTotal, 6);
// reduce to concatenated keys
srKeys = structReduce(sr, function(acc, k, v) {
    return acc & k;
}, "");
assert("structReduce concat keys length", len(srKeys), 3);

// ============================================================
// StructSome / StructEvery (from Lucee structSome.cfc, structEvery.cfc)
// ============================================================
sso = {a:1, b:2, c:3};
assertTrue("structSome finds match", structSome(sso, function(k, v) {
    return v > 2;
}));
assertFalse("structSome no match", structSome(sso, function(k, v) {
    return v > 10;
}));

sev = {a:2, b:4, c:6};
assertTrue("structEvery all even", structEvery(sev, function(k, v) {
    return v mod 2 == 0;
}));
sev2 = {a:2, b:3, c:6};
assertFalse("structEvery not all even", structEvery(sev2, function(k, v) {
    return v mod 2 == 0;
}));

// ============================================================
// StructGet (from Lucee StructGet.cfc)
// ============================================================
// structGet(path) RESOLVES the path, and only creates it when it is missing.
// These two assertions alone passed against an implementation that returned a
// fresh empty struct for EVERY input and never touched a scope (GH #346) — the
// path here does not exist, so "empty struct" was right for the wrong reason.
// Everything below distinguishes the two.
sg = structGet("_sgTest.nested.deep");
assertTrue("structGet returns struct", isStruct(sg));
assertTrue("structGet returns empty struct", structIsEmpty(sg));

// ...and the miss CREATED the path, so it is now addressable.
assertTrue("structGet created the missing path", isDefined("_sgTest.nested.deep"));
// The struct handed back IS the stored one, so the documented get-or-create
// idiom (`s = structGet(p); s.k = v`) writes through.
sg.marker = "written";
assertTrue("structGet result writes through", isDefined("_sgTest.nested.deep.marker"));
assert("structGet write-through value", _sgTest.nested.deep.marker, "written");

// An EXISTING path resolves to its value rather than to an empty struct.
_sgDeep = { nested = 1 };
_sgTwo  = { a = { b = 2 } };
_sgArr  = [ 10, 20 ];
assert("structGet resolves a scalar leaf", structGet("_sgDeep.nested"), 1);
assert("structGet resolves a nested scalar", structGet("_sgTwo.a.b"), 2);
assertTrue("structGet resolves a struct", isStruct(structGet("_sgTwo.a")));
assert("structGet struct leaf key", structGet("_sgTwo.a").b, 2);
// A scope-qualified path and a bracket subscript both resolve.
assert("structGet honours a scope prefix", structGet("variables._sgDeep.nested"), 1);
assert("structGet honours a bracket index", structGet("_sgArr[1]"), 10);
assertTrue("structGet returns an array as-is", isArray(structGet("_sgArr")));
// A path THROUGH a scalar has nowhere to put the rest of itself.
assertThrows("structGet cannot create through a scalar", function() {
    return structGet("_sgDeep.nested.x");
});
assertThrows("structGet rejects an empty name", function() {
    return structGet("");
});

// ============================================================
// StructValueArray (from Lucee structValueArray.cfc)
// ============================================================
sva = {a:1, b:2, c:3};
svaResult = structValueArray(sva);
assert("structValueArray length", arrayLen(svaResult), 3);
// values should contain 1, 2, 3 (order may vary)
svaSum = 0;
for (val in svaResult) {
    svaSum = svaSum + val;
}
assert("structValueArray values sum", svaSum, 6);

// empty struct
assert("structValueArray empty", arrayLen(structValueArray({})), 0);

// ============================================================
// StructKeyTranslate (from Lucee StructKeyTranslate.cfc)
// ============================================================
// structKeyTranslate lowercases keys in-place
skt = {MyKey:1, AnotherKey:2};
structKeyTranslate(skt);
assertTrue("structKeyTranslate has lowercased key", structKeyExists(skt, "mykey"));
assertTrue("structKeyTranslate has second key", structKeyExists(skt, "anotherkey"));
assert("structKeyTranslate preserves value", skt.mykey, 1);

// ============================================================
// Duplicate - deep copy (from Lucee Duplicate.cfc)
// ============================================================
dup1 = {a:{b:1, c:2}, d:"hello"};
dup2 = duplicate(dup1);
assert("duplicate preserves nested value", dup2.a.b, 1);
assert("duplicate preserves string value", dup2.d, "hello");
// deep copy: modifying nested struct in copy does not affect original
dup2.a.b = 99;
assert("duplicate deep copy - original unchanged", dup1.a.b, 1);
assert("duplicate deep copy - copy changed", dup2.a.b, 99);
// modifying top-level in copy does not affect original
dup2.d = "modified";
assert("duplicate top-level independent", dup1.d, "hello");

suiteEnd();
</cfscript>
