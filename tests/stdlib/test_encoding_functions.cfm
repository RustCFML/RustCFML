<cfscript>
suiteBegin("Encoding/Decoding Functions");

// ========================================
// charsetDecode for text-to-binary (UTF-8/US-ASCII)
// ========================================
binData = charsetDecode("hello", "utf-8");
assertTrue("charsetDecode utf-8 returns binary", isBinary(binData));
assert("charsetDecode utf-8 round-trip", charsetEncode(binData, "utf-8"), "hello");

binAscii = charsetDecode("test", "us-ascii");
assertTrue("charsetDecode us-ascii returns binary", isBinary(binAscii));
assert("charsetDecode us-ascii round-trip", charsetEncode(binAscii, "us-ascii"), "test");

// binaryDecode hex (standard encoding)
binHex = binaryDecode("48656C6C6F", "hex");
assertTrue("binaryDecode hex returns binary", isBinary(binHex));
assert("binaryDecode hex round-trip", charsetEncode(binHex, "utf-8"), "Hello");

// binaryDecode base64 (standard encoding)
binB64 = binaryDecode("SGVsbG8=", "base64");
assertTrue("binaryDecode base64 returns binary", isBinary(binB64));
assert("binaryDecode base64 round-trip", charsetEncode(binB64, "utf-8"), "Hello");

// ========================================
// binaryEncode
// ========================================
binInput = charsetDecode("Hello", "utf-8");
hexEncoded = binaryEncode(binInput, "hex");
assert("binaryEncode hex", hexEncoded, "48656C6C6F");

b64Encoded = binaryEncode(binInput, "base64");
assert("binaryEncode base64", b64Encoded, "SGVsbG8=");

// ========================================
// charsetDecode / charsetEncode
// ========================================
csBin = charsetDecode("Hello World", "utf-8");
assertTrue("charsetDecode returns binary", isBinary(csBin));

csStr = charsetEncode(csBin, "utf-8");
assert("charsetEncode round-trip", csStr, "Hello World");

csBinIso = charsetDecode("test", "iso-8859-1");
assertTrue("charsetDecode iso-8859-1 returns binary", isBinary(csBinIso));
csStrIso = charsetEncode(csBinIso, "iso-8859-1");
assert("charsetEncode iso round-trip", csStrIso, "test");

// ========================================
// encodeForHTMLAttribute
// ========================================
htmlAttr = encodeForHTMLAttribute('<div class="test">');
assertTrue("encodeForHTMLAttribute encodes lt", find("&lt;", htmlAttr) > 0);
// OWASP ESAPI immune set for an HTML-attribute context is `, . - _` — those
// inert chars pass through unencoded. (Engine-agnostic: Adobe/BoxLang/RustCFML
// follow OWASP and leave them alone; Lucee 7 encodes almost nothing, so it also
// leaves them alone — the assert holds on both. We deliberately do NOT assert
// that `/` IS encoded here, since that genuinely diverges between engines.)
immune = encodeForHTMLAttribute("a.b,c-d_e");
assert("encodeForHTMLAttribute leaves immune set , . - _ untouched", immune, "a.b,c-d_e");

// ========================================
// encodeForXML
// ========================================
xmlEnc = encodeForXML('<tag attr="val">');
assertTrue("encodeForXML encodes lt", find("&lt;", xmlEnc) > 0);
assertTrue("encodeForXML encodes gt", find("&gt;", xmlEnc) > 0);

// ========================================
// encodeForXMLAttribute
// ========================================
xmlAttr = encodeForXMLAttribute('<tag>' & chr(9) & chr(10));
assertTrue("encodeForXMLAttribute encodes lt", find("&lt;", xmlAttr) > 0);

// ========================================
// encodeForHTML / encodeForURL / encodeForJavaScript / encodeForCSS
// ========================================
efHtml = encodeForHTML("<b>bold</b>");
assertTrue("encodeForHTML encodes lt", find("&lt;", efHtml) > 0);

// GH #283: encodeForURL and urlEncodedFormat have DIFFERENT space encodings.
// encodeForURL uses form-encoding semantics (ESAPI / java.net.URLEncoder), so a
// space is `+` — matching Lucee 5/6/7, Adobe CF and BoxLang. urlEncodedFormat /
// urlEncode use `%20` (GH #270). The two must not share one encoder.
efUrl = encodeForURL("hello world");
assert("encodeForURL encodes space as +", efUrl, "hello+world");
assert("urlEncodedFormat encodes space as %20", urlEncodedFormat("hello world"), "hello%20world");
// A '+' in the input round-trips to %2B on both.
assert("encodeForURL escapes a literal plus", encodeForURL("a b+c"), "a+b%2Bc");
assert("urlEncodedFormat escapes a literal plus", urlEncodedFormat("a b+c"), "a%20b%2Bc");

efJs = encodeForJavaScript("alert()");
assertTrue("encodeForJavaScript returns string", len(efJs) > 0);

efCss = encodeForCSS("<div>");
assertTrue("encodeForCSS returns string", len(efCss) > 0);

// ========================================
// decodeForHTML
// ========================================
decoded = decodeForHTML("&lt;b&gt;bold&lt;/b&gt;");
assert("decodeForHTML basic entities", decoded, "<b>bold</b>");

decoded2 = decodeForHTML("&amp;");
assert("decodeForHTML amp", decoded2, "&");

decoded3 = decodeForHTML("&quot;");
assert("decodeForHTML quot", decoded3, chr(34));

decoded4 = decodeForHTML("&##39;");
assert("decodeForHTML numeric apos", decoded4, chr(39));

decodedNum = decodeForHTML("&##65;&##66;&##67;");
assert("decodeForHTML numeric entities", decodedNum, "ABC");

decodedHex = decodeForHTML("&##x41;&##x42;&##x43;");
assert("decodeForHTML hex entities", decodedHex, "ABC");

// ========================================
// decodeFromURL
// ========================================
urlDecoded = decodeFromURL("hello%20world");
assert("decodeFromURL basic", urlDecoded, "hello world");

urlDecoded2 = decodeFromURL("hello+world");
assert("decodeFromURL plus sign", urlDecoded2, "hello world");

urlDecoded3 = decodeFromURL("%3C%3E%26");
assert("decodeFromURL special chars", urlDecoded3, "<>&");

// ========================================
// urlEncode
// ========================================
urlEnc = urlEncode("hello world");
// GH #270: space encodes as %20 (Lucee/ACF), not `+`.
assertTrue("urlEncode encodes space as %20", find("%20", urlEnc) > 0);

urlEnc2 = urlEncode("a&b=c");
assertTrue("urlEncode encodes amp", find("%26", urlEnc2) > 0);

// ========================================
// canonicalize
// ========================================
canon1 = canonicalize("%3Cscript%3E", false, false);
assert("canonicalize URL-encoded", canon1, "<script>");

canon2 = canonicalize("&lt;script&gt;", false, false);
assert("canonicalize HTML-encoded", canon2, "<script>");

canon3 = canonicalize("hello", false, false);
assert("canonicalize plain text unchanged", canon3, "hello");

// Double-encoded
canon4 = canonicalize("%26lt%3B", false, false);
assert("canonicalize double-encoded", canon4, "<");

// GitHub #252: ESAPI JavaScriptCodec decode pass (backslash escapes). Built
// with chr() so the CFML source carries no ambiguous escape sequences.
jsBS = chr(92);   // backslash
jsQ  = chr(34);   // double quote
assert("canonicalize JS escaped quote",   canonicalize("a" & jsBS & jsQ & "b", false, false), "a" & jsQ & "b");
assert("canonicalize JS hex escape",      canonicalize(jsBS & "x41", false, false), "A");
assert("canonicalize JS unicode escape",  canonicalize(jsBS & "u0041", false, false), "A");
assert("canonicalize JS backslash-backslash", canonicalize(jsBS & jsBS, false, false), jsBS);
// HTML-entity + percent passes still match (regression guard).
assert("canonicalize html+percent still works", canonicalize("a&##x20;b %41", false, false), "a b A");
// The Wheels attribute pipeline shape: JVM engines produce &quot; (the un-decoded
// backslash was previously entity-encoded to &##x5c;&quot;).
assert("attr pipeline EncodeForHTMLAttribute(Canonicalize())",
       encodeForHTMLAttribute(canonicalize("btn" & jsBS & jsQ & "x", false, false)), "btn&quot;x");

suiteEnd();
</cfscript>
