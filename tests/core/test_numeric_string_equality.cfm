<cfscript>
// GH #398. `eq` / `==` / `is` between two NUMERIC strings compares them as
// numbers on Lucee and ACF; we compared them as text, so "000" neq "000000".
// TestBox's Assertion.equalize() routes numeric comparisons through this
// operator, and any code comparing zero-padded ids, versions or colour codes
// diverged. Every expectation was measured on Lucee 7.1.0.204.
suiteBegin("Numeric string equality");

assertTrue("zero-padded zeroes are equal", "000" eq "000000");
assertTrue("leading zeroes do not change the value", "01" eq "1");
assertTrue("a trailing .0 does not change the value", "1.0" eq "1");
assertTrue("exponent notation compares numerically", "1e2" eq "100");
assertTrue("surrounding whitespace is trimmed", " 1" eq "1");
assertTrue("trailing whitespace too", "1 " eq "1");
assertTrue("sign prefixes compare numerically", "+1" eq "1");
assertTrue("negative zero is zero", "-0" eq "0");
assertTrue("scale differences compare numerically", "0.1" eq "00.10");
assertTrue("== agrees", "000" == "000000");
assertTrue("is agrees", "01" is "1");
assertFalse("and neq is its inverse", "000" neq "000000");

// Non-numeric strings keep the case-insensitive TEXT comparison.
assertTrue("non-numeric strings compare case-insensitively", "abc" eq "ABC");
assertTrue("a hex colour code is not numeric, so compares as text", "0066FF" eq "0066ff");
assertFalse("a thousands separator is not numeric syntax", "1,0" eq "1");
assertFalse("0x notation is not numeric to CFML", "0x10" eq "16");
assertFalse("an empty string is not zero", "" eq "0");

// The IEEE special spellings parse as floats in Rust but are NOT numeric to
// CFML — letting them through would have made these two equal.
assertFalse("INF is not numeric", isNumeric("INF"));
assertFalse("Infinity is not numeric", isNumeric("Infinity"));
assertFalse("NaN is not numeric", isNumeric("NaN"));
assertFalse("so INF does not equal Infinity", "INF" eq "Infinity");

// Numeric-looking forms Lucee DOES accept.
assertTrue("a bare decimal point is numeric", isNumeric(".5"));
assertTrue("a trailing decimal point is numeric", isNumeric("5."));
assertTrue("padded whitespace is numeric", isNumeric("  12  "));
assertFalse("an underscore separator is not", isNumeric("1_000"));

// Boolean-literal strings keep their own coercion.
assertTrue("true still equals 1", "true" eq "1");

// The sharp edge of the rule, worth pinning: a hex string can be numeric.
// "0E576548" is zero times ten to the 576548 — numerically zero — so it equals
// "00000000" on Lucee and here. Code comparing hex must use compare().
assertTrue("an E-form hex block is numeric", isNumeric("0E576548"));
assertTrue("so it equals a zeroed block", "0E576548" eq "00000000");
assert("while compare() still sees two different strings",
	compare("0E576548", "00000000"), 1);

// The ordering operators were already numeric-aware; they must stay that way.
assertTrue("lt compares numerically", "01" lt "2");
assertTrue("gt compares numerically", "10" gt "9");

// formatBaseN reports lowercase digits, as Lucee does.
assert("formatBaseN uses lowercase digits", formatBaseN(255, 16), "ff");
assert("and is unchanged for digit-only output", formatBaseN(102, 16), "66");

suiteEnd();
</cfscript>
