<cfscript>
suiteBegin("Regex: malformed patterns and Perl class syntax");

// A malformed pattern throws in every re* BIF, as on Lucee. It used to answer
// "no match": reFind 0, reMatch [], and reReplace returned the subject
// unchanged, so the bug never surfaced at the call site.
assertThrows("reFind malformed pattern throws", function() { reFind("[a", "abc"); });
assertThrows("reFindNoCase malformed pattern throws", function() { reFindNoCase("[a", "abc"); });
assertThrows("reReplace malformed pattern throws", function() { reReplace("abc", "[a", "x"); });
assertThrows("reReplaceNoCase malformed pattern throws", function() { reReplaceNoCase("abc", "(a", "x"); });
assertThrows("reMatch malformed pattern throws", function() { reMatch("[a", "abc"); });
assertThrows("reMatchNoCase malformed pattern throws", function() { reMatchNoCase("a)", "abc"); });
// isValid("regex", …) still answers false rather than throwing (Lucee agrees).
assertFalse("isValid regex with a malformed pattern is false", isValid("regex", "abc", "[a"));

// Perl (Lucee's engine) reads `[` inside a class as a literal. Rust's regex
// reads it as a nested class, so these failed to compile and never matched.
bb = "Hello [b]bold[/b] and [i]it[/i]";
assert("BBCode with [^[]* replaces", reReplace(bb, "\[(b|bold)\]([^[]*)\[/\1\]", "<b>\2</b>", "all"), "Hello <b>bold</b> and [i]it[/i]");
assert("[^[]* without a backslash elsewhere", reFind("x[^[]y", "axzy"), 2);
assert("class holding ( [ ] )", reReplace("f(a)[b]", "[([\])]", "", "all"), "fab");
assert("] first in a negated class is literal", reReplace("[url=http://x]go[/url]", "\[url=([^]]*)\]", "<\1>"), "<http://x>go[/url]");
assert("POSIX class still works", reReplace("a1b2", "[[:digit:]]", "", "all"), "ab");
assert("POSIX class beside a literal [", reReplace("a1[b", "[[:digit:][]", "", "all"), "ab");
// `\x` with one hex digit, as Perl allows.
assert("\x with one hex digit in a class", reFind("[\xA]", "ab" & chr(10) & "c"), 3);
assert("\x with one hex digit outside a class", reFind("\xA", "ab" & chr(10)), 3);
assert("\x with two hex digits unchanged", reFind("\x41", "zA"), 2);

suiteEnd();
</cfscript>
