<cfscript>
suiteBegin("replaceNoCase / replaceListNoCase: Unicode and empty-substring edges");

// replaceNoCase used to lowercase the whole string and reuse those byte offsets
// on the ORIGINAL. `İ` lowercases to two characters (3 bytes vs 2), so every
// offset after it was wrong: a slice landed off a char boundary and panicked
// the engine, which no CFML try/catch could catch. Expected values below were
// all taken from Lucee 7.1.

assert("a match after a length-changing character", replaceNoCase("İx", "x", "y"), "İy");
assert("scope=all after length-changing characters", replaceNoCase("İİab", "A", "Z", "all"), "İİZb");
assert("a pattern that equals the lowercased form does not match", replaceNoCase("aİb", "i" & chr(775), "-"), "aİb");
assert("İ does not match i", replaceNoCase("İx", "i", "y"), "İx");
assert("dotless ı matches I", replaceNoCase("ıx", "I", "y"), "yx");
assert("dotless ı matches i", replaceNoCase("ıx", "i", "y"), "yx");
assert("long s matches s", replaceNoCase(chr(383) & "x", "s", "y"), "yx");
assert("Kelvin sign does not match K", replaceNoCase(chr(8490) & "x", "K", "y"), chr(8490) & "x");
assert("capital sharp s matches ß", replaceNoCase("ß", chr(7838), "y"), "y");
assert("ß does not match SS", replaceNoCase("Straße", "SS", "x"), "Straße");
assert("accented letters fold", replaceNoCase("Éx", "é", "y"), "yx");
assert("Greek sigma folds, final form included", replaceNoCase("ΣΑΣ", "σ", "s", "all"), "sΑs");
assert("astral characters are kept whole", replaceNoCase("a😀B😀b", "b", "-", "all"), "a😀-😀-");
assert("matches do not overlap", replaceNoCase("aaaa", "AA", "b", "all"), "bb");
assert("scope is case-insensitive", replaceNoCase("aAa", "a", "-", "ALL"), "---");
assert("scope=one replaces only the first", replaceNoCase("aAa", "a", "-", "ONE"), "-Aa");

assertThrows("an empty substring is refused", function() {
	replaceNoCase("abc", "", "x");
});
assert("plain replace() with an empty substring is a no-op", replace("abc", "", "x", "all"), "abc");
assert("plain replace() scope=one with an empty substring is a no-op", replace("abc", "", "x"), "abc");

assert("replaceListNoCase after a length-changing character", replaceListNoCase("İx,İy", "X,Y", "1,2"), "İ1,İ2");
assert("replaceListNoCase skips empty list items", replaceListNoCase("abc", "b,,c", "1,2,3"), "a12");
assert("replaceListNoCase basic", replaceListNoCase("Hello World", "HELLO,world", "Hi,Earth"), "Hi Earth");

suiteEnd();
</cfscript>
