<cfscript>
suiteBegin("encodeForJavaScript follows ESAPI; hash() honours encoding and numIterations");

// ESAPI JavaScriptCodec (Lucee's own tests expect \x22\x27 for a double+single quote).
assert("quotes", encodeForJavaScript("""'"), "\x22\x27");
assert("letters, digits and ,._ pass through", encodeForJavaScript("aZ09,._"), "aZ09,._");
assert("ampersand and entity text are encoded",
	encodeForJavaScript("&##39;);alert(1)//"),
	"\x26\x2339\x3B\x29\x3Balert\x281\x29\x2F\x2F");
assert("backtick and template-literal syntax are encoded", encodeForJavaScript("`${x}`"), "\x60\x24\x7Bx\x7D\x60");
assert("Latin-1 uses \x", encodeForJavaScript("é"), "\xE9");
bs = chr(92);
assert("above 0xFF uses \u", encodeForJavaScript(chr(8364)), bs & "u20AC");
assert("astral characters encode as their UTF-16 halves", encodeForJavaScript(chr(128512)), bs & "uD83D" & bs & "uDE00");

// hash(input, algorithm, encoding, numIterations): the digest BYTES are
// re-digested numIterations times (Lucee Hash.invoke).
one = hash("abc", "SHA-256");
assert("one iteration is the plain digest", hash("abc", "SHA-256", "UTF-8", 1), one);
assert("two iterations digest the first digest's bytes",
	hash("abc", "SHA-256", "UTF-8", 2),
	hash(binaryDecode(one, "hex"), "SHA-256"));
assertTrue("1000 iterations differ from one", hash("abc", "SHA-512", "UTF-8", 1000) != hash("abc", "SHA-512"));
assert("encoding changes the bytes hashed",
	hash("é", "MD5", "ISO-8859-1"),
	hash(charsetDecode("é", "ISO-8859-1"), "MD5"));
assert("zero iterations is treated as one", hash("abc", "MD5", "UTF-8", 0), hash("abc", "MD5"));

suiteEnd();
</cfscript>
