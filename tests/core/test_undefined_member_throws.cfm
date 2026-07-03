<cfscript>
suiteBegin("Core: reading an undefined struct/scope member throws (Lucee/ACF parity)");

// Background (cross-engine contract):
// Reading a member that does not exist — `struct.missingKey`, `local.missing`,
// or a component's `variables.missing` — must THROW "Variable ... is undefined",
// exactly as a bare undefined identifier does. RustCFML previously returned Null
// silently for these three cases, which broke the standard CFML lazy-init idiom:
//
//   function getRenderer() {
//       try { return variables._renderer; }   // undefined on first call
//       catch (any e) { variables._renderer = build(); }
//       return variables._renderer;
//   }
//
// On Lucee the `return variables._renderer` THROWS, the catch fires, and the
// renderer is lazily built. When the read returns Null instead, the catch never
// runs and getRenderer() returns null (Preside/ColdBox HandlerService then
// 500s with "Variable 'renderer' is undefined"). The tolerant idioms — `?:`,
// isNull(), null-safe `?.`, structKeyExists() — must still read a miss as Null.

// ---- 1. bare reads of a genuinely-missing member THROW ----
plainStruct = { hello: "world" };
assertThrows("struct.missingKey read throws", function() {
	var boom = plainStruct.missingKey;
	return boom;
});

function localMissProbe() {
	// local.missing has no such key in this frame's local scope
	return local.missing;
}
assertThrows("local.missing read throws", localMissProbe);

// ---- 2. the lazy-init-by-exception idiom works on a component ----
lazyObj = new core.LazyRenderer();
assert("lazy-init idiom: first call catches the undefined read and builds", lazyObj.getRenderer(), "BUILT");
assert("lazy-init idiom: second call returns the cached value", lazyObj.getRenderer(), "BUILT");

// a bare read of a never-set component member still throws
assertThrows("component variables.missing bare read throws", function() {
	return lazyObj.readMissing();
});

// ---- 3. tolerant idioms still read a miss as Null (no throw) ----
assert("elvis on missing member yields the default", (plainStruct.missingKey ?: "DEF"), "DEF");
assertTrue("isNull() on a missing member is true", isNull(plainStruct.missingKey));
assertFalse("structKeyExists on a missing member is false", structKeyExists(plainStruct, "missingKey"));
assertTrue("null-safe read of a missing link is null", isNull(plainStruct.deep?.x));
assert("elvis on a deep missing chain yields the default", (plainStruct.a.b ?: "D2"), "D2");

// ---- 4. auto-vivification of nested writes still works (relies on tolerant base reads) ----
autoBracket = {};
autoBracket["a"]["b"] = "viv";
assert("nested bracket write auto-vivifies", autoBracket.a.b, "viv");

autoDot = {};
autoDot.x.y = "vd";
assert("nested dot write auto-vivifies", autoDot.x.y, "vd");

// ---- 5. a present member with a genuine value still reads fine ----
assert("present member reads its value", plainStruct.hello, "world");

suiteEnd();
</cfscript>
