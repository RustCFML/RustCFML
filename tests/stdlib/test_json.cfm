<cfscript>
suiteBegin("JSON Functions");

// --- serializeJSON ---
assert("serializeJSON integer", serializeJSON(42), "42");
assert("serializeJSON string", serializeJSON("hello"), '"hello"');
assert("serializeJSON boolean", serializeJSON(true), "true");

jsonArr = serializeJSON([1, 2, 3]);
assertTrue("serializeJSON array is valid JSON", isJSON(jsonArr));
assertTrue("serializeJSON array contains brackets", find("[", jsonArr) > 0);

jsonObj = serializeJSON({name: "test"});
assertTrue("serializeJSON struct is valid JSON", isJSON(jsonObj));
assertTrue("serializeJSON struct contains brace", find("{", jsonObj) > 0);

// --- deserializeJSON ---
assert("deserializeJSON integer", deserializeJSON("42"), 42);
assert("deserializeJSON string", deserializeJSON('"hello"'), "hello");

parsedArr = deserializeJSON("[1,2,3]");
assertTrue("deserializeJSON array isArray", isArray(parsedArr));
assert("deserializeJSON array length", arrayLen(parsedArr), 3);

parsedObj = deserializeJSON('{"name":"test"}');
assertTrue("deserializeJSON struct has key", structKeyExists(parsedObj, "name"));
assert("deserializeJSON struct value", parsedObj.name, "test");

// --- isJSON ---
assertTrue("isJSON number", isJSON("42"));
assertTrue("isJSON array", isJSON("[1,2,3]"));
assertTrue("isJSON object", isJSON('{"a":1}'));
assertFalse("isJSON invalid", isJSON("not json {"));

// --- round-trip ---
original = {a: 1, b: [2, 3]};
roundTrip = deserializeJSON(serializeJSON(original));
assert("round-trip struct key a", roundTrip.a, 1);
assertTrue("round-trip struct key b is array", isArray(roundTrip.b));
assert("round-trip array length", arrayLen(roundTrip.b), 2);

// --- circular references must NOT overflow the native stack (GitHub #178) ---
// Reference-typed structs/arrays can alias and form cycles (e.g. a TestBox mock
// holds this.mockBox, whose generator holds the mock back). Before the fix this
// recursed until the process aborted with an uncatchable SIGABRT. The cycle is
// broken with null so serialization stays total and non-crashing.
circStruct = {}; circStruct.name = "root"; circStruct.self = circStruct;
circJson = serializeJSON(circStruct);
assertTrue("circular struct serializes without crashing", len(circJson) > 0);
assertTrue("circular struct keeps non-cyclic data", findNoCase('"name":"root"', circJson) > 0);

circArr = []; arrayAppend(circArr, "x"); arrayAppend(circArr, circArr);
circArrJson = serializeJSON(circArr);
assertTrue("circular array serializes without crashing", len(circArrJson) > 0);

// --- lenient (Lucee/ACF-compatible) deserializeJSON ---
// Lucee's deserializeJSON accepts unquoted keys, single-quoted strings/keys,
// trailing commas, and // and /* */ comments. Strict serde_json rejected these,
// which broke Preside's AdHocTaskManagerService.getProgress() (a DB `result`
// column of "{ test:'this' }"). Verified against Lucee 7.0.4.
lenientUnquotedKey = deserializeJSON("{ test:'this' }");
assert("lenient: unquoted key + single-quote value", lenientUnquotedKey.test, "this");

lenientSingleKey = deserializeJSON("{'a':1}");
assert("lenient: single-quoted key", lenientSingleKey.a, 1);

lenientTrailingObj = deserializeJSON("{a:1,}");
assert("lenient: trailing comma in object", lenientTrailingObj.a, 1);

lenientTrailingArr = deserializeJSON("[1,2,]");
assert("lenient: trailing comma in array", arrayLen(lenientTrailingArr), 2);

assert("lenient: top-level single-quoted string", deserializeJSON("'single'"), "single");

lenientBlockComment = deserializeJSON("{a:/*c*/1}");
assert("lenient: block comment", lenientBlockComment.a, 1);

lenientLineComment = deserializeJSON("//line#chr(10)#{a:1}");
assert("lenient: line comment", lenientLineComment.a, 1);

lenientEscQuote = deserializeJSON("{a:'he\'llo'}");
assert("lenient: escaped single-quote in single-quoted string", lenientEscQuote.a, "he'llo");

// Still strict where Lucee is strict: a missing comma between members must throw.
assertThrows("lenient parser still rejects missing comma", function() {
	deserializeJSON("{a:1 b:2}");
});

// Strict, valid JSON continues to parse (fast path unchanged).
strictStill = deserializeJSON('{"x":1,"y":[true,null,2.5]}');
assert("strict path: object key", strictStill.x, 1);
assertTrue("strict path: nested array", isArray(strictStill.y));

// isJSON is lenient in Lucee too — true for the lenient forms, false only for
// genuinely-malformed JSON. Verified against Lucee 7.0.4.
assertTrue("isJSON lenient: unquoted key", isJSON("{ test:'this' }"));
assertTrue("isJSON lenient: single-quoted key", isJSON("{'a':1}"));
assertTrue("isJSON lenient: trailing comma", isJSON("[1,2,]"));
assertTrue("isJSON lenient: top-level single-quoted string", isJSON("'single'"));
assertFalse("isJSON still false on missing comma", isJSON("{a:1 b:2}"));

suiteEnd();
</cfscript>
