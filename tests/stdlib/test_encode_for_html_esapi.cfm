<cfscript>
suiteBegin("EncodeForHTML full ESAPI codec");

// EncodeForHTML must apply the OWASP/ESAPI HTMLEntityCodec for an HTML *content*
// context: every char below U+0100 is encoded except the immune set `, . - _`
// and space; named entity where one exists, else a lowercase hex numeric entity.
// RustCFML previously encoded only 6 chars (& < > " ' /) and left `(`, `)`, `=`,
// etc. raw — wrong by OWASP/Adobe/BoxLang and broke Wheels form helpers.

// Engine-agnostic truths (hold on Lucee too): named entities + immune passthrough.
assert("ampersand", EncodeForHTML("a&b"), "a&amp;b");
assert("angle brackets", EncodeForHTML("<x>"), "&lt;x&gt;");
// ESAPI (RustCFML, Adobe) names the quote; Lucee 7.0/7.1 use the OWASP Java
// Encoder, which writes it as a decimal reference.
assert("double quote", EncodeForHTML("a""b"), isRustCFML() ? "a&quot;b" : "a&##34;b");
assert("alnum untouched", EncodeForHTML("abcXYZ123"), "abcXYZ123");
assert("immune chars + space untouched", EncodeForHTML("a, b.c-d_e"), "a, b.c-d_e");

// RustCFML / Adobe / BoxLang OWASP behaviour — Lucee 7 may differ here, so guard.
// (Literal `#` in the expected entities must be escaped as `##` in CFML strings.)
if (isRustCFML()) {
	assert("parens encoded", EncodeForHTML("alert(1)"), "alert&##x28;1&##x29;");
	assert("apostrophe encoded", EncodeForHTML("it's"), "it&##x27;s");
	assert("slash encoded", EncodeForHTML("a/b"), "a&##x2f;b");
	assert("equals encoded", EncodeForHTML("a=b"), "a&##x3d;b");
	// The classic XSS payload, fully neutralised.
	assert("xss payload", EncodeForHTML("alert(""XSS"")"), "alert&##x28;&quot;XSS&quot;&##x29;");
}

suiteEnd();
</cfscript>
