<cfscript>
suiteBegin("String Functions");

// --- len ---
assert("len of hello", len("hello"), 5);

// --- ucase / lcase ---
assert("ucase", ucase("hello"), "HELLO");
assert("lcase", lcase("HELLO"), "hello");

// --- trim / ltrim / rtrim ---
assert("trim", trim("  hi  "), "hi");
assert("ltrim", ltrim("  hi"), "hi");
assert("rtrim", rtrim("hi  "), "hi");

// --- find / findNoCase ---
assert("find ll in hello", find("ll", "hello"), 3);
assert("findNoCase LL in hello", findNoCase("LL", "hello"), 3);

// --- mid / left / right ---
assert("mid(hello,2,3)", mid("hello", 2, 3), "ell");
assert("left(hello,3)", left("hello", 3), "hel");
assert("right(hello,3)", right("hello", 3), "llo");

// left() and right() with negative counts (GitHub #504)
testString = "/a.b";
assert("left(/a.b, -5)", left(testString, -5), "/a.b");
assert("left(/a.b, -4)", left(testString, -4), "/a.b");
assert("left(/a.b, -3)", left(testString, -3), "/");
assert("left(/a.b, -2)", left(testString, -2), "/a");
assert("left(/a.b, -1)", left(testString, -1), "/a.");
assert("left(/a.b, 1)", left(testString, 1), "/");
assert("left(/a.b, 2)", left(testString, 2), "/a");
assert("left(/a.b, 3)", left(testString, 3), "/a.");
assert("left(/a.b, 4)", left(testString, 4), "/a.b");
assert("left(/a.b, 5)", left(testString, 5), "/a.b");
assert("right(/a.b, -5)", right(testString, -5), "/a.b");
assert("right(/a.b, -4)", right(testString, -4), "/a.b");
assert("right(/a.b, -3)", right(testString, -3), "b");
assert("right(/a.b, -2)", right(testString, -2), ".b");
assert("right(/a.b, -1)", right(testString, -1), "a.b");
assert("right(/a.b, 1)", right(testString, 1), "b");
assert("right(/a.b, 2)", right(testString, 2), ".b");
assert("right(/a.b, 3)", right(testString, 3), "a.b");
assert("right(/a.b, 4)", right(testString, 4), "/a.b");
assert("right(/a.b, 5)", right(testString, 5), "/a.b");
assertThrows("left() with count=0 throws", function() { left("test", 0); });
assertThrows("right() with count=0 throws", function() { right("test", 0); });

// left() and right() with multibyte characters and negative counts
mbString = "café";
assert("left(café, -1)", left(mbString, -1), "caf");
assert("left(café, -2)", left(mbString, -2), "ca");
assert("left(café, 3)", left(mbString, 3), "caf");
assert("right(café, -1)", right(mbString, -1), "afé");
assert("right(café, -2)", right(mbString, -2), "fé");
assert("right(café, 3)", right(mbString, 3), "afé");

// --- replace / replaceNoCase ---
assert("replace ll with r", replace("hello", "ll", "r"), "hero");
assert("replaceNoCase hello->world", replaceNoCase("Hello", "hello", "world"), "world");

// --- reverse ---
assert("reverse hello", reverse("hello"), "olleh");

// --- repeatString ---
assert("repeatString ab 3", repeatString("ab", 3), "ababab");

// --- insert ---
assert("insert X into hello at 3", insert("X", "hello", 3), "helXlo");

// --- removeChars ---
assert("removeChars hello 2 3", removeChars("hello", 2, 3), "ho");
assert("removeChars last char", removeChars("abc", 3, 1), "ab");
assert("removeChars count past the end", removeChars("abc", 2, 10), "a");
// An out-of-range start panicked the request; Lucee throws an expression error.
assertThrows("removeChars start past the end throws", function() { removeChars("abc", 10, 1); });
assertThrows("removeChars start 0 throws", function() { removeChars("abc", 0, 1); });
assertThrows("removeChars on empty string throws", function() { removeChars("", 1, 1); });
assertThrows("removeChars negative count throws", function() { removeChars("abc", 2, -1); });

// --- chr / asc ---
assert("chr(65)", chr(65), "A");
assert("asc(A)", asc("A"), 65);

// --- compare / compareNoCase ---
assert("compare equal", compare("abc", "abc"), 0);
assert("compareNoCase equal", compareNoCase("abc", "ABC"), 0);

// --- ucFirst ---
assert("ucFirst hello world", ucFirst("hello world"), "Hello world");

// --- wrap ---
wrapped = wrap("hello world test long text", 5);
assertTrue("wrap contains line break", find(chr(10), wrapped) > 0);

// --- spanIncluding ---
assert("spanIncluding", spanIncluding("hello123", "helo"), "hello");

// --- getToken ---
assert("getToken second", getToken("one,two,three", 2, ","), "two");

suiteEnd();
</cfscript>
