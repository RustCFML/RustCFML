<cfscript>
suiteBegin("Base64/Hex Codec Edge Cases");

// These lock down the exact semantics of the base64 and hex codecs, which were
// rewritten for speed: the decoders used to search the 64-byte alphabet
// LINEARLY for every character (~32 comparisons per char), making toBinary()
// ~8x slower than toBase64() on the same data. The rewrite uses a 256-entry
// reverse table, and everything below is behaviour that had to survive it —
// including the tolerant handling of malformed input.

// ========================================
// Padding: 0, 1 and 2 trailing pad chars
// ========================================
assert("base64 round trip, no padding",  toBase64(toBinary("QUJD")), "QUJD");     // "ABC"
assert("base64 round trip, one pad",     toBase64(toBinary("QUI=")), "QUI=");     // "AB"
assert("base64 round trip, two pads",    toBase64(toBinary("QQ==")), "QQ==");     // "A"
assert("base64 decode length, no pad",   len(toBinary("QUJD")), 3);
assert("base64 decode length, one pad",  len(toBinary("QUI=")), 2);
assert("base64 decode length, two pads", len(toBinary("QQ==")), 1);

// ========================================
// MIME-wrapped base64: newlines, CRs and spaces are skipped, and grouping is
// positional over the FILTERED sequence (so a break mid-quad still decodes).
// ========================================
assert("base64 skips LF between quads",   len(toBinary("QUJD" & chr(10) & "REVG")), 6);
assert("base64 skips CRLF between quads", len(toBinary("QUJD" & chr(13) & chr(10) & "REVG")), 6);
assert("base64 skips spaces",             len(toBinary("QUJD REVG")), 6);
assert("base64 skips LF mid-quad",        len(toBinary("QU" & chr(10) & "JD")), 3);
assert("wrapped base64 decodes to same bytes",
       toBase64(toBinary("QUJD" & chr(10) & "REVG")), "QUJDREVG");

// ========================================
// Malformed input stays tolerant (never throws): an unknown character decodes
// as a zero sextet, and a trailing lone character is dropped.
// ========================================
assert("base64 lone trailing char dropped", len(toBinary("QUJDR")), 3);
assert("base64 empty string",               len(toBinary("")), 0);
// Both engines return binary rather than throwing; the byte COUNT differs, so
// only the no-throw contract is asserted (see the divergence note below).
assertTrue("base64 unknown char does not throw", isBinary(toBinary("QU*D")));

// ========================================
// Full alphabet, including the +/ pair and every 6-bit value
// ========================================
allAlpha = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
assert("base64 full alphabet round trips", toBase64(toBinary(allAlpha)), allAlpha);

// ========================================
// binaryEncode/binaryDecode agree with toBase64/toBinary
// ========================================
raw = charsetDecode("The quick brown fox jumps over the lazy dog", "utf-8");
assert("binaryEncode base64 == toBase64", binaryEncode(raw, "base64"), toBase64(raw));
assert("binaryDecode base64 round trip",
       charsetEncode(binaryDecode(toBase64(raw), "base64"), "utf-8"),
       "The quick brown fox jumps over the lazy dog");

// ========================================
// Hex: uppercase output, even/odd length, non-hex tolerance
// ========================================
assert("binaryEncode hex is uppercase",  binaryEncode(binaryDecode("deadbeef", "hex"), "hex"), "DEADBEEF");
assert("hex round trip",                 binaryEncode(binaryDecode("DEADBEEF", "hex"), "hex"), "DEADBEEF");
assert("hex decode length",              len(binaryDecode("DEADBEEF", "hex")), 4);
assert("hex all zero bytes",             binaryEncode(binaryDecode("0000", "hex"), "hex"), "0000");
assert("hex 0x0F low nibble kept",       binaryEncode(binaryDecode("0F", "hex"), "hex"), "0F");
assert("hex round trip via encode first", len(binaryDecode(binaryEncode(raw, "hex"), "hex")), len(raw));

// ========================================
// objectSave/objectLoad through the ColdBox DiskStore shape: base64 string ->
// toBinary -> objectLoad. This is the exact path that was ~8x too slow.
// ========================================
payload  = { html="<div>content</div>", nested={ n=42, list=[1,2,3] } };
restored = objectLoad(toBinary(toBase64(objectSave(payload))));
assert("objectSave/Load via base64 keeps string",  restored.html, "<div>content</div>");
assert("objectSave/Load via base64 keeps number",  restored.nested.n, 42);
assert("objectSave/Load via base64 keeps array",   arrayLen(restored.nested.list), 3);
assert("objectLoad accepts binary directly",       objectLoad(objectSave("plain")), "plain");

// A blob big enough to cross several thousand base64 quads, so the sized
// allocation and the quad loop are exercised at scale rather than on 4 chars.
bigBlob  = repeatString("0123456789abcdef", 4096);   // 64KB
bigRound = objectLoad(toBinary(toBase64(objectSave({ b=bigBlob }))));
assert("64KB blob survives base64 round trip", bigRound.b, bigBlob);

// ========================================
// urlEncodedFormat: the percent-escape path was rewritten to use a hex table
// instead of allocating per character. Multi-byte UTF-8 must still emit one
// %XX per BYTE, uppercase.
// ========================================
assert("urlEncodedFormat space",        urlEncodedFormat("a b"), "a%20b");
assert("urlEncodedFormat slash",        urlEncodedFormat("a/b"), "a%2Fb");
assert("urlEncodedFormat alnum kept",   urlEncodedFormat("Az09"), "Az09");
assert("urlEncodedFormat multibyte",    urlEncodedFormat(chr(8364)), "%E2%82%AC");   // euro sign

// Deliberately NOT asserted here, because RustCFML and Lucee 7.0.5.41 disagree
// and the divergences predate the codec rewrite (see docs/known-issues.md):
//   * urlEncodedFormat("-_.*")        RustCFML keeps them; Lucee escapes them.
//   * encodeForURL(" ")               RustCFML "+"; Lucee "%20".
//   * binaryDecode("DEADBEE","hex")   RustCFML drops the nibble; Lucee throws.
//   * binaryDecode("DEADBEZZ","hex")  RustCFML decodes 0; Lucee throws.
//   * toBinary("QU*D")                RustCFML 3 bytes; Lucee 2.
// Asserting either behaviour here would freeze one engine's answer as correct.

suiteEnd();
</cfscript>
