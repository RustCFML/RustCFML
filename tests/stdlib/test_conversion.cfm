<cfscript>
suiteBegin("Type Conversion Functions");

// --- toString ---
assert("toString integer", toString(42), "42");
assert("toString boolean true", toString(true), "true");

arrStr = arrayToList([1, 2, 3]);
assert("arrayToList of [1,2,3]", arrStr, "1,2,3");

// --- val ---
assert("val numeric string", val("42"), 42);
assert("val mixed string", val("123abc"), 123);
assert("val non-numeric", val("abc"), 0);
assert("val empty string", val(""), 0);

// --- javacast ---
assert("javacast int from string", javacast("int", "42"), 42);
assert("javacast string from int", javacast("string", 42), "42");

// --- int ---
assert("int truncates down", int(3.9), 3);
assert("int on negative", int(-3.9), -4);

// --- yesNoFormat ---
assert("yesNoFormat true", yesNoFormat(true), "Yes");
assert("yesNoFormat false", yesNoFormat(false), "No");

// --- booleanFormat ---
bfTrue = booleanFormat(true);
assertTrue("booleanFormat true is truthy", bfTrue);
// Lucee parity: booleanFormat is the true/false form, not Yes/No (that is yesNoFormat).
assert("booleanFormat true is 'true'", booleanFormat(true), "true");
assert("booleanFormat false is 'false'", booleanFormat(false), "false");
assert("booleanFormat 1 is 'true'", booleanFormat(1), "true");

// --- incrementValue / decrementValue ---
assert("incrementValue", incrementValue(5), 6);
assert("decrementValue", decrementValue(5), 4);

suiteEnd();
</cfscript>
