<cfscript>
suiteBegin("Struct Member Functions");

// --- count ---
assert("struct.count()", {a: 1, b: 2}.count(), 2);

// --- isEmpty ---
assertFalse("struct.isEmpty() with keys", {a: 1, b: 2}.isEmpty());
assertTrue("empty struct.isEmpty()", {}.isEmpty());

// --- keyExists ---
assertTrue("struct.keyExists(a)", {a: 1, b: 2}.keyExists("a"));
assertFalse("struct.keyExists(z)", {a: 1, b: 2}.keyExists("z"));

// --- keyList ---
// Struct key order is not guaranteed; check that keyList contains expected keys
kl = {a: 1, b: 2}.keyList();
assertTrue("struct.keyList() contains A", findNoCase("A", kl) > 0);
assertTrue("struct.keyList() contains B", findNoCase("B", kl) > 0);

// --- keyArray ---
assert("struct.keyArray().len()", {a: 1, b: 2}.keyArray().len(), 2);

// --- insert (mutating) ---
s1 = {a: 1};
s1.insert("b", 2);
assert("struct.insert() then count", s1.count(), 2);

// --- delete (mutating) ---
s2 = {a: 1, b: 2};
s2.delete("a");
assert("struct.delete() then count", s2.count(), 1);

// --- find ---
assert("struct.find(a)", {a: 1, b: 2}.find("a"), 1);

// --- copy ---
s3 = {a: 1, b: 2};
s3copy = s3.copy();
assert("struct.copy().count()", s3copy.count(), 2);

// --- append ---
s4 = {a: 1};
s4.append({b: 2});
assert("struct.append() then count", s4.count(), 2);

// --- get (java.util.Map member passthrough, GH #223) ---
s5 = { myKey: "123", Other: 7 };
assert("struct.get() returns value", s5.get("myKey"), "123");
assert("struct.get() is case-insensitive", s5.get("OTHER"), 7);
assertTrue("struct.get() missing key is null", isNull(s5.get("nope")));

// --- user-defined function member shadows built-in member function ---
// A struct key holding a closure whose name collides with a built-in member
// function (filter/map/each/sort/append/len/reduce) must dispatch to the
// stored closure, not the built-in HOF (Lucee parity). TestBox 2.8 relies on
// this: it stores a `filter` closure in a struct and invokes it via
// `arguments.directory.filter( path )`.
sc = {
	  filter = function( p ){ return "filter:" & arguments.p; }
	, map    = function( p ){ return "map:" & arguments.p; }
	, each   = function( p ){ return "each:" & arguments.p; }
	, sort   = function( p ){ return "sort:" & arguments.p; }
	, append = function( p ){ return "append:" & arguments.p; }
	, len    = function( p ){ return "len:" & arguments.p; }
	, reduce = function( p ){ return "reduce:" & arguments.p; }
};
assert("struct closure member 'filter' shadows built-in", sc.filter("x"), "filter:x");
assert("struct closure member 'map' shadows built-in", sc.map("x"), "map:x");
assert("struct closure member 'each' shadows built-in", sc.each("x"), "each:x");
assert("struct closure member 'sort' shadows built-in", sc.sort("x"), "sort:x");
assert("struct closure member 'append' shadows built-in", sc.append("x"), "append:x");
assert("struct closure member 'len' shadows built-in", sc.len("x"), "len:x");
assert("struct closure member 'reduce' shadows built-in", sc.reduce("x"), "reduce:x");
// method-call in argument position (the exact TestBox failure shape)
assert("shadowing works in argument position", "R=" & sc.filter("y"), "R=filter:y");

// A struct WITHOUT a colliding member still gets the built-in HOF.
hof = { a: 1, b: 2 };
doubled = hof.map( function( k, v ){ return v * 2; } );
assert("built-in struct.map() still works when no member collides", doubled.b, 4);

suiteEnd();
</cfscript>
