<cfscript>
suiteBegin("encodeForJavaScript follows ESAPI; hash() honours encoding and numIterations");

// ESAPI JavaScriptCodec (Lucee's own tests expect \x22\x27 for a double+single quote).
assert("quotes", encodeForJavaScript("""'"), "\x22\x27");
assert("letters, digits and ,._ pass through", encodeForJavaScript("aZ09,._"), "aZ09,._");
// RustCFML-only from here: RustCFML keeps ESAPI's JavaScriptCodec (as ACF and
// Lucee up to Guard/ESAPI extension 3.0 do). Lucee 7.1.2 ships Guard 3.1.0.1,
// whose encodeForJavaScript is the OWASP Java Encoder: it leaves `;()` and
// non-ASCII text as they are, writes `/` as `\/`, and DROPS astral characters
// (an emoji encodes to ""). Deliberate divergence, tracked in GH #453.
if (isRustCFML()) {
	assert("ampersand and entity text are encoded",
		encodeForJavaScript("&##39;);alert(1)//"),
		"\x26\x2339\x3B\x29\x3Balert\x281\x29\x2F\x2F");
	assert("backtick and template-literal syntax are encoded", encodeForJavaScript("`${x}`"), "\x60\x24\x7Bx\x7D\x60");
	assert("Latin-1 uses \x", encodeForJavaScript("é"), "\xE9");
	bs = chr(92);
	assert("above 0xFF uses \u", encodeForJavaScript(chr(8364)), bs & "u20AC");
	assert("astral characters encode as their UTF-16 halves", encodeForJavaScript(chr(128512)), bs & "uD83D" & bs & "uDE00");
}

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
<cfscript>
suiteBegin("generateSecretKey validates Blowfish key sizes like the JCE");
assertThrows("Blowfish size below 32", function() { generateSecretKey("BLOWFISH", 4); });
assertThrows("Blowfish size not a multiple of 8", function() { generateSecretKey("BLOWFISH", 100); });
assertThrows("Blowfish size above 448", function() { generateSecretKey("BLOWFISH", 512); });
assert("Blowfish 128 is 16 bytes", len(binaryDecode(generateSecretKey("BLOWFISH", 128), "base64")), 16);
// The JCE default, as Lucee/ACF return it: 256 bits on Java 21.
assert("AES default is 32 bytes", len(binaryDecode(generateSecretKey("AES"), "base64")), 32);
assert("AES 128 is 16 bytes", len(binaryDecode(generateSecretKey("AES", 128), "base64")), 16);
suiteEnd();
</cfscript>
