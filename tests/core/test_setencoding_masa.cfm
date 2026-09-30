<cfscript>
suiteBegin("setEncoding() (Lucee BIF surfaced booting Masa CMS)");

// Masa/Mura onRequestStart calls setEncoding("url","utf-8") /
// setEncoding("form","utf-8"). It sets the charset used to read the URL/FORM
// scope. RustCFML already decodes request data as UTF-8, so a utf-8 call is an
// identity operation and must not corrupt existing scope values.

url.plain    = "hello";
url.accented = "é";          // U+00E9 — UTF-8 bytes C3 A9
form.plain   = "world";

// utf-8 is identity: values are unchanged.
setEncoding("url", "utf-8");
setEncoding("form", "utf-8");
assert("setEncoding url utf-8 leaves ascii untouched",     url.plain,    "hello");
assert("setEncoding url utf-8 leaves accented untouched",  url.accented, "é");
assert("setEncoding form utf-8 leaves ascii untouched",    form.plain,   "world");

// Returns an empty string (Lucee 7.0/7.1), not null.
assert("setEncoding returns an empty string", setEncoding("url", "utf-8"), "");

// A latin1 re-decode reinterprets the UTF-8 bytes under ISO-8859-1: the two
// bytes of "é" (C3 A9) become the two Latin-1 codepoints Ã (C3) and © (A9).
// RustCFML-only: Lucee re-reads the scope from the raw query string instead, so
// a key set in code (like this one) is simply gone afterwards. For real request
// data the two agree.
if ( isRustCFML() ) {
	url.reinterp = "é";
	setEncoding("url", "iso-8859-1");
	assert("setEncoding latin1 reinterprets utf-8 bytes", url.reinterp, "Ã©");
}

// Only URL and FORM are settable: anything else throws (Lucee 7.0/7.1) and
// must not touch the URL scope.
assertThrows("setEncoding rejects the cookie scope", function() { setEncoding("cookie", "iso-8859-1"); });
assertThrows("setEncoding rejects the session scope", function() { setEncoding("session", "utf-8"); });
assert("setEncoding on a non url/form scope leaves url untouched", url.plain, "hello");
assert("scope names are case-insensitive", setEncoding("URL", "utf-8"), "");

suiteEnd();
</cfscript>
