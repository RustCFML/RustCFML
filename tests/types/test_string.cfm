<cfscript>
suiteBegin("Type: String");

// --- String literals (double quotes) ---
s = "hello world";
assert("double-quoted string", s, "hello world");

// --- Single-quoted strings ---
sq = 'single quoted';
assert("single-quoted string", sq, "single quoted");

// --- Empty string ---
empty = "";
assert("empty string", empty, "");
assert("empty string length", len(empty), 0);

// --- String concatenation with & ---
first = "Hello";
second = " World";
assert("string concatenation", first & second, "Hello World");

// --- String comparison is case-insensitive ---
assert("compare() case-insensitive eq", compare("abc", "abc"), 0);
assertTrue("case-insensitive equality", "abc" EQ "ABC");
assertTrue("case-insensitive EQ keyword", "Hello" EQ "hello");

// --- String length with len() ---
assert("len of string", len("abcdef"), 6);
assert("len of single char", len("x"), 1);

// --- String with special characters ---
special = "it's a ""test""";
assert("double-quote escaping", len(special) > 0, true);

// --- String numeric coercion ---
numStr = "42";
assert("string + number coercion", numStr + 0, 42);

// --- String interpolation with #expr# ---
name = "World";
interpolated = "Hello #name#!";
assert("string interpolation variable", interpolated, "Hello World!");
calcInterp = "Result: #1 + 2#";
assert("string interpolation expression", calcInterp, "Result: 3");

// --- Multiline string ---
multi = "line1
line2";
assertTrue("multiline string has content", len(multi) > 5);

// --- string bracket-index is 1-based CHARACTER access (Lucee/ACF/BoxLang) ---
sidx = "hello";
assert("string[1] returns first char", sidx[1], "h");
assert("string[5] returns last char", sidx[5], "o");
// works through a struct member then index (the Preside EmailService shape:
// a single-recipient `to` string validated via `sendArgs.to[1]`)
sst = { to = "to@test.com" };
assert("member string index", sst.to[1], "t");
// multibyte is char-based, not byte-based
assert("string index is char-based for multibyte", "héllo"[2], chr(233));
// out-of-range / zero subscripts throw a catchable error
assertThrows("string[out-of-range] throws", function(){ var x = "hi"[9]; });
assertThrows("string[0] throws", function(){ var x = "hi"[0]; });

suiteEnd();
</cfscript>
