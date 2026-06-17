<cfscript>
suiteBegin("String Member Functions");

// --- len ---
assert("string.len()", "hello".len(), 5);

// --- ucase / lcase ---
assert("string.ucase()", "hello".ucase(), "HELLO");
assert("string.lcase()", "HELLO".lcase(), "hello");

// --- trim / ltrim / rtrim ---
assert("string.trim()", "  hi  ".trim(), "hi");
assert("string.ltrim()", "  hi".ltrim(), "hi");
assert("string.rtrim()", "hi  ".rtrim(), "hi");

// --- left / right / mid ---
assert("string.left(3)", "hello".left(3), "hel");
assert("string.right(3)", "hello".right(3), "llo");
assert("string.mid(2,3)", "hello".mid(2, 3), "ell");

// --- reverse ---
assert("string.reverse()", "hello".reverse(), "olleh");

// --- find / findNoCase ---
assert("string.find(ll)", "hello".find("ll"), 3);
assert("string.findNoCase(LL)", "hello".findNoCase("LL"), 3);

// --- replace ---
assert("string.replace(ll, r)", "hello".replace("ll", "r"), "hero");

// --- repeatString ---
assert("string.repeatString(3)", "hello".repeatString(3), "hellohellohello");
assert("ab.repeatString(3)", "ab".repeatString(3), "ababab");

// --- insert ---
assert("string.insert(X, 3)", "hello".insert("X", 3), "helXlo");

// --- chaining: ucase then reverse ---
assert("chain ucase().reverse()", "hello".ucase().reverse(), "OLLEH");

// --- chaining: trim then ucase ---
assert("chain trim().ucase()", "  hello  ".trim().ucase(), "HELLO");

// --- ucFirst ---
assert("string.ucFirst()", "hello world".ucFirst(), "Hello world");

// --- compare ---
assert("string.compare() equal", "Hello".compare("Hello"), 0);

// --- toString ---
assert("string.toString() returns receiver text", "hello".toString(), "hello");

// --- getToken (member form; string-first signature, no arg swap) ---
// Regression: the member form returned empty while the standalone getToken()
// worked. WireBox's delegate shorthand parser uses `item.getToken(1, "=")`.
assert("string.getToken(index) default whitespace delim", "a b c".getToken(2), "b");
assert("string.getToken(index, delim) first", "Worker=vacation".getToken(1, "="), "Worker");
assert("string.getToken(index, delim) second", "Worker=vacation".getToken(2, "="), "vacation");
assert("string.getToken keeps left when no delim present", "ram2>memory".getToken(1, "="), "ram2>memory");

suiteEnd();
</cfscript>
