<cfscript>
suiteBegin("Regex \b inside a character class");

// In Java/Lucee/ACF regex, `\b` INSIDE a character class `[...]` means the
// backspace char (U+0008); outside a class it's a word boundary. The Rust regex
// crate rejects `\b` inside a class as a compile error, and RustCFML's regex
// callers silently swallow compile errors as "no match" — so a pattern like
// Wheels autoLink's `[^\s\b]+` became a complete no-op (URLs never linked).
// RustCFML now rewrites in-class `\b` to `\x08` before compiling.

// The autoLink-style pattern matches non-whitespace runs.
assert("reFind [^\s\b]+ matches", reFind("[^\s\b]+", "abc"), 1);
assert("reFind [^\s\b]+ skips leading space", reFind("[^\s\b]+", " abc"), 2);
assert("reReplace [^\s\b]+ all", reReplace("a b c", "[^\s\b]+", "X", "all"), "X X X");

// A real URL inside surrounding text (the autoLink trigger).
assertTrue("URL run found via [^\s\b]+", reFind("(https?://[^\s\b]+)", "go http://wheels.dev now") GT 0);

// Word-boundary \b OUTSIDE a class is unaffected.
assert("word boundary \b still works", reFind("\bfoo\b", "a foo b"), 3);

suiteEnd();

// ---------------------------------------------------------------------------
suiteBegin("Regex literal hyphen adjacent to a class shorthand");

// A literal `-` placed next to a class shorthand (\w \d \s …) inside a char
// class is treated as a literal hyphen by Java/Lucee/PCRE, but the Rust regex
// crate parses `\w-\.` as a range with non-literal endpoints and rejects it,
// which used to make the whole pattern a no-op. Preside's resource-URI
// validator `[\w][\w-\.]*\:...` relied on this, so every getResource() lookup
// short-circuited to its default. RustCFML now escapes the hyphen to `\-`.

uri = "core.master:test.resource.key";
assertTrue("Preside resource-URI validator matches", reFind("[\w][\w-\.]*\:[\w][\w-\.]*[^\.]", uri) GT 0);
assertTrue("[\w-\.] hyphen-after-shorthand matches", reFind("[\w-\.]", uri) GT 0);
assertTrue("[\d-x] hyphen-after-shorthand matches", reFind("[\d-x]", "7") GT 0);

// Genuine ranges must keep working (the hyphen is NOT escaped there).
assert("range [a-z] preserved", reFind("[a-z]", "ABCxyz"), 4);
assert("range [0-9] preserved", reFind("[0-9]", "ab12"), 3);
// Leading/escaped hyphens unchanged.
assertTrue("leading hyphen literal [-a]", reFind("[-a]", "x-y") GT 0);

suiteEnd();

// ---------------------------------------------------------------------------
suiteBegin("Regex \< and \> are literal angle brackets (not word boundaries)");

// Java/Lucee/ACF regex has no `\<` / `\>` word-boundary escapes (word
// boundaries are `\b`); `\<` and `\>` are escaped LITERAL angle brackets. The
// Rust `regex` crate added `\<`/`\>` as start/end-of-word boundaries, which
// silently changed the match set. Wheels formsdateplainSpec asserts
// `ReMatchNoCase("\<option", html)` counts the OPENING <option> tags only —
// with the word-boundary reading it matched every "option" WORD (opening AND
// closing tags), so minuteStep=15 reported 8 options instead of 4.
html = '<option value="0">00</option><option value="15">15</option>';
assert("reMatch \<option counts literal opens", arrayLen(reMatchNoCase("\<option", html)), 2);
assert("reMatch <option (bare) same as \<option", arrayLen(reMatchNoCase("<option", html)), 2);
assert("reMatch option\> counts literal closes-ish", arrayLen(reMatchNoCase("option\>", html)), 2);
assert("reFind \<option at start", reFindNoCase("\<option", html), 1);
// \b (real word boundary) is unaffected by this change.
assert("\boption\b still a word boundary", arrayLen(reMatchNoCase("\boption\b", html)), 4);

suiteEnd();
</cfscript>
