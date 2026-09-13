<cfscript>
// GH #393. The 5th argument, scope="all", must return EVERY match. We dropped
// it and returned the single-match result, so callers that iterated the result
// silently found nothing — Preside's PresideObjectService._getObfuscationsInSql
// never entered its loop and left nested obfuscated SQL params unprefixed.
//
// Every expectation here was measured on Lucee 7.1.0.204.
suiteBegin("reFind scope=all");

s = "abcabd";

// With sub-expressions: an ARRAY of the same struct the single-match form returns.
all = ReFindNoCase("b(.)", s, 1, true, "all");
assertTrue("scope=all with subexpressions returns an array", isArray(all));
assert("both matches are reported", arrayLen(all), 2);
assert("first match position", all[1].pos[1], 2);
assert("first match text", all[1].match[1], "bc");
assert("first capture group", all[1].match[2], "c");
assert("second match position", all[2].pos[1], 5);
assert("second match text", all[2].match[1], "bd");

// Without sub-expressions: an ARRAY of 1-based positions.
positions = ReFindNoCase("b(.)", s, 1, false, "all");
assertTrue("scope=all without subexpressions returns an array", isArray(positions));
assert("position count", arrayLen(positions), 2);
assert("first position", positions[1], 2);
assert("second position", positions[2], 5);

// The case-sensitive twin takes the same path.
assert("reFind honours scope=all too", arrayLen(ReFind("b(.)", s, 1, true, "all")), 2);

// `start` still applies in scope=all — matches before it are not reported.
fromThree = ReFind("b(.)", s, 3, false, "all");
assert("start offset skips the earlier match", arrayLen(fromThree), 1);
assert("only the later match remains", fromThree[1], 5);

// Lucee's two no-match answers are ASYMMETRIC, and callers check the shape
// before iterating: WITH sub-expressions it is an array holding one zero-struct;
// WITHOUT, it is the plain scalar 0 — NOT an empty array.
noneSub = ReFind("z(.)", s, 1, true, "all");
assertTrue("no match with subexpressions is still an array", isArray(noneSub));
assert("holding exactly one entry", arrayLen(noneSub), 1);
assert("whose position is zero", noneSub[1].pos[1], 0);
assertFalse("no match without subexpressions is the scalar 0",
	isArray(ReFind("z(.)", s, 1, false, "all")));
assert("and equals zero", ReFind("z(.)", s, 1, false, "all"), 0);

// A zero-width match must still advance, by one character, and Lucee reports a
// hit at every position INCLUDING one past the end of the string.
empty = ReFind("x*", s, 1, false, "all");
assert("zero-width matches cover every position plus one", arrayLen(empty), len(s) + 1);
assert("starting at 1", empty[1], 1);
assert("and ending one past the end", empty[arrayLen(empty)], len(s) + 1);

// Anything other than "all" is the single-match scope, unchanged.
assertFalse("scope=one returns the single-match struct", isArray(ReFind("b(.)", s, 1, true, "one")));
assertFalse("an absent scope is unchanged", isArray(ReFind("b(.)", s, 1, true)));

suiteEnd();
</cfscript>
