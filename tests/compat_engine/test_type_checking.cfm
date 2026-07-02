<cfscript>
// Lucee 7 Compatibility Tests: Type Checking
// Synthesized from https://github.com/lucee/Lucee/tree/7.0/test
// Original tests Copyright (c) 2014, the Railo Company LLC / Copyright (c) 2015-2016, Lucee Association Switzerland
// Licensed under the GNU Lesser General Public License v2.1
// Adapted for RustCFML test harness

// ============================================================
// isNull (from Lucee IsNull.cfc)
// ============================================================
suiteBegin("Lucee7: isNull");
assertTrue("isNull on nullValue()", isNull(nullValue()));
assertFalse("isNull on string", isNull("hello"));
assertFalse("isNull on number", isNull(42));
assertFalse("isNull on boolean", isNull(true));
assertFalse("isNull on empty string", isNull(""));
assertFalse("isNull on zero", isNull(0));
assertFalse("isNull on empty array", isNull([]));
assertFalse("isNull on empty struct", isNull({}));
suiteEnd();

// ============================================================
// isEmpty (from Lucee IsEmpty.cfc)
// ============================================================
suiteBegin("Lucee7: isEmpty");
assertTrue("isEmpty empty string", isEmpty(""));
assertFalse("isEmpty non-empty string", isEmpty("hello"));
assertTrue("isEmpty empty array", isEmpty([]));
assertFalse("isEmpty non-empty array", isEmpty([1]));
assertTrue("isEmpty empty struct", isEmpty({}));
assertFalse("isEmpty non-empty struct", isEmpty({a:1}));
assertFalse("isEmpty non-empty struct check", isEmpty({a:1}));
suiteEnd();

// ============================================================
// isSimpleValue (from Lucee IsSimpleValue.cfc)
// ============================================================
suiteBegin("Lucee7: isSimpleValue");
assertTrue("isSimpleValue string", isSimpleValue("hello"));
assertTrue("isSimpleValue number", isSimpleValue(1));
assertTrue("isSimpleValue boolean", isSimpleValue(true));
assertTrue("isSimpleValue empty string", isSimpleValue(""));
assertTrue("isSimpleValue float", isSimpleValue(1.5));
assertFalse("isSimpleValue array", isSimpleValue([]));
assertFalse("isSimpleValue struct", isSimpleValue({}));
assertFalse("isSimpleValue query", isSimpleValue(queryNew("x")));
suiteEnd();

// ============================================================
// isArray (from Lucee IsArray.cfc)
// ============================================================
suiteBegin("Lucee7: isArray");
assertTrue("isArray empty array", isArray([]));
assertTrue("isArray with elements", isArray([1,2,3]));
assertFalse("isArray struct", isArray({}));
assertFalse("isArray string", isArray("hello"));
assertFalse("isArray number", isArray(42));
assertFalse("isArray boolean", isArray(true));
suiteEnd();

// ============================================================
// isStruct (from Lucee IsStruct.cfc)
// ============================================================
suiteBegin("Lucee7: isStruct");
assertTrue("isStruct empty struct", isStruct({}));
assertTrue("isStruct with keys", isStruct({a:1, b:2}));
assertFalse("isStruct array", isStruct([]));
assertFalse("isStruct string", isStruct("hello"));
assertFalse("isStruct number", isStruct(42));
suiteEnd();

// ============================================================
// isQuery (from Lucee IsQuery.cfc)
// ============================================================
suiteBegin("Lucee7: isQuery");
assertTrue("isQuery on query", isQuery(queryNew("x")));
assertTrue("isQuery on query with data", isQuery(queryNew("name,age","varchar,integer",[["John",30]])));
assertFalse("isQuery on struct", isQuery({}));
assertFalse("isQuery on array", isQuery([]));
assertFalse("isQuery on string", isQuery("hello"));
suiteEnd();

// ============================================================
// isBoolean (from Lucee IsBoolean.cfc)
// ============================================================
suiteBegin("Lucee7: isBoolean");
assertTrue("isBoolean true", isBoolean(true));
assertTrue("isBoolean false", isBoolean(false));
assertTrue("isBoolean yes string", isBoolean("yes"));
assertTrue("isBoolean no string", isBoolean("no"));
assertTrue("isBoolean true string", isBoolean("true"));
assertTrue("isBoolean false string", isBoolean("false"));
assertTrue("isBoolean 1", isBoolean(1));
assertTrue("isBoolean 0", isBoolean(0));
assertFalse("isBoolean hello", isBoolean("hello"));
assertFalse("isBoolean array", isBoolean([]));
suiteEnd();

// ============================================================
// isNumeric (from Lucee IsNumeric.cfc)
// ============================================================
suiteBegin("Lucee7: isNumeric");
assertTrue("isNumeric integer", isNumeric(1));
assertTrue("isNumeric string integer", isNumeric("123"));
assertTrue("isNumeric float string", isNumeric("1.5"));
assertTrue("isNumeric negative", isNumeric("-3.5"));
assertTrue("isNumeric zero", isNumeric(0));
assertFalse("isNumeric alpha string", isNumeric("abc"));
assertFalse("isNumeric empty string", isNumeric(""));
assertFalse("isNumeric mixed", isNumeric("12abc"));
suiteEnd();

// ============================================================
// isDate (from Lucee IsDate.cfc)
// ============================================================
suiteBegin("Lucee7: isDate");
assertTrue("isDate on now()", isDate(now()));
assertTrue("isDate ISO string", isDate("2024-01-01"));
assertTrue("isDate full datetime", isDate("2024-06-15 10:30:00"));
assertFalse("isDate random string", isDate("hello"));
assertFalse("isDate empty string", isDate(""));
suiteEnd();

// ============================================================
// isBinary (from Lucee IsBinary.cfc)
// ============================================================
suiteBegin("Lucee7: isBinary");
assertTrue("isBinary on binary value", isBinary(toBinary(toBase64("test"))));
assertFalse("isBinary on string", isBinary("hello"));
assertFalse("isBinary on number", isBinary(42));
assertFalse("isBinary on array", isBinary([]));
suiteEnd();

// ============================================================
// isJSON (from Lucee IsJson.cfc)
// ============================================================
suiteBegin("Lucee7: isJSON");
assertTrue("isJSON object", isJSON('{"a":1}'));
assertTrue("isJSON array", isJSON("[1,2,3]"));
assertTrue("isJSON string value", isJSON('"hello"'));
assertTrue("isJSON number", isJSON("42"));
assertTrue("isJSON boolean", isJSON("true"));
assertTrue("isJSON null", isJSON("null"));
assertFalse("isJSON invalid", isJSON("not json"));
assertFalse("isJSON empty string", isJSON(""));
assertTrue("isJSON nested", isJSON('{"users":[{"name":"John"}]}'));
suiteEnd();

// ============================================================
// isClosure / isCustomFunction (from Lucee IsClosure.cfc, IsCustomFunction.cfc)
// ============================================================
suiteBegin("Lucee7: isClosure / isCustomFunction");
myClosure = function(){ return 1; };
assertTrue("isCustomFunction on closure", isCustomFunction(myClosure));
assertFalse("isCustomFunction on string", isCustomFunction("hello"));
assertFalse("isCustomFunction on number", isCustomFunction(42));
assertFalse("isCustomFunction on struct", isCustomFunction({}));
assertFalse("isClosure on string", isClosure("hello"));
assertFalse("isClosure on number", isClosure(42));
suiteEnd();

// ============================================================
// isDefined (from Lucee IsDefined.cfc)
// ============================================================
suiteBegin("Lucee7: isDefined");
myDefinedVar = 1;
assertTrue("isDefined existing var", isDefined("myDefinedVar"));
assertFalse("isDefined non-existing var", isDefined("undeclaredVar123xyz"));
myStr = "hello";
assertTrue("isDefined string var", isDefined("myStr"));
suiteEnd();

// ============================================================
// isValid (from Lucee IsValid.cfc - comprehensive)
// ============================================================
suiteBegin("Lucee7: isValid");

// numeric
assertTrue("isValid numeric int", isValid("numeric", 123));
assertTrue("isValid numeric float", isValid("numeric", 1.5));
assertTrue("isValid numeric string", isValid("numeric", "42"));
assertFalse("isValid numeric alpha", isValid("numeric", "abc"));

// boolean
assertTrue("isValid boolean true", isValid("boolean", true));
assertTrue("isValid boolean yes", isValid("boolean", "yes"));
assertFalse("isValid boolean hello", isValid("boolean", "hello"));

// email
assertTrue("isValid email good", isValid("email", "test@example.com"));
assertFalse("isValid email bad", isValid("email", "notanemail"));
assertFalse("isValid email empty", isValid("email", ""));

// url
assertTrue("isValid url https", isValid("url", "https://example.com"));
assertTrue("isValid url http", isValid("url", "http://example.com/path"));

// integer
assertTrue("isValid integer 42", isValid("integer", 42));
assertTrue("isValid integer string", isValid("integer", "100"));
assertFalse("isValid integer float", isValid("integer", 1.5));

// string
assertTrue("isValid string hello", isValid("string", "hello"));
assertTrue("isValid string empty", isValid("string", ""));
assertTrue("isValid string number", isValid("string", 123));

// array / struct / query
assertTrue("isValid array", isValid("array", []));
assertFalse("isValid array on struct", isValid("array", {}));
assertTrue("isValid struct", isValid("struct", {}));
assertFalse("isValid struct on array", isValid("struct", []));
assertTrue("isValid query", isValid("query", queryNew("x")));
assertFalse("isValid query on struct", isValid("query", {}));

// time — bare time-of-day or any parseable date/datetime (Lucee parity)
assertTrue("isValid time 12h", isValid("time", "6:15 PM"));
assertTrue("isValid time 24h", isValid("time", "18:15"));
assertTrue("isValid time padded", isValid("time", "06:15"));
assertTrue("isValid time with seconds", isValid("time", "06:15:30"));
assertTrue("isValid time accepts a date", isValid("time", "2020-01-01"));
assertFalse("isValid time rejects out-of-range", isValid("time", "25:99"));
assertFalse("isValid time rejects words", isValid("time", "hello"));

// uuid
assertTrue("isValid uuid", isValid("uuid", createUUID()));

// regex (3-arg form: checks if value matches a regex pattern)
assertTrue("isValid regex match", isValid("regex", "hello", "^[a-z]+$"));
assertFalse("isValid regex no match", isValid("regex", "HELLO", "^[a-z]+$"));

// xml — delegates to IsXml (Lucee/ACF). Was always false, breaking TestBox
// .toBeXML() on Wheels renderWith's valid XML (controller.renderingSpec).
assertTrue("isValid xml with decl", isValid("xml", '<?xml version="1.0"?><root><a>1</a></root>'));
assertTrue("isValid xml no decl", isValid("xml", "<root><a>1</a></root>"));
assertFalse("isValid xml on plain text", isValid("xml", "not xml at all"));
assertFalse("isValid xml on malformed", isValid("xml", "<root><a>1</root>"));

// json — delegates to IsJson (Lucee/ACF).
assertTrue("isValid json object", isValid("json", '{"a":1}'));
assertTrue("isValid json array", isValid("json", '[1,2,3]'));
assertFalse("isValid json bad", isValid("json", "{a:}"));

suiteEnd();
</cfscript>
