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


// A numeric string against a NUMBER compares by value too, whatever the
// string's spelling. Only an integer spelling matched an Int before, so
// "1.00" eq 1 was false while "1.00" gte 1 and "1.00" lte 1 were both true.
// A SQL driver hands a DECIMAL column back as "1.00", so any code testing such
// a column against an integer literal failed on every row. Measured on Lucee
// 7.0.4.34.
assertTrue("a decimal string equals the integer it denotes", "1.00" eq 1);
assertTrue("in either operand order", 1 eq "1.00");
assertTrue("a decimal string equals a decimal number", "1.50" eq 1.5);
assertTrue("exponent notation equals the integer", "1e2" eq 100);
assertTrue("whitespace around a decimal string is trimmed", " 1.0 " eq 1);
assertTrue("a sign prefix on a decimal string", "+1.0" eq 1);
assertTrue("negative decimals", "-2.50" eq -2.5);
assertTrue("a zero decimal string equals zero", "0.00" eq 0);
assertTrue("== agrees with a number", "1.00" == 1);
assertTrue("is agrees with a number", "1.00" is 1);
assertFalse("neq is the inverse against a number", "1.00" neq 1);
assertFalse("a different value is still different", "1.01" eq 1);
q = queryNew("flag", "varchar", [["1.00"], ["0.00"]]);
assertTrue("a query cell holding 1.00 equals 1", q.flag[1] eq 1);
assertTrue("a query cell holding 0.00 equals 0", q.flag[2] eq 0);
assertTrue("equality agrees with gte", ("1.00" eq 1) eq ("1.00" gte 1 and "1.00" lte 1));
sw = "default";
switch ("1.00") { case 1: sw = "one"; break; }
assert("switch matches a decimal string to an integer case", sw, "one");
assertTrue("integer spellings still match", "007" eq 7);
assertFalse("trailing text is not numeric", "1.00x" eq 1);
assertFalse("an empty string is not zero against a number", "" eq 0);
assertFalse("0x notation is not numeric against a number", "0x10" eq 16);
assertFalse("a thousands separator is not numeric against a number", "1,000" eq 1000);
assertFalse("INF is not numeric against a number", "INF" eq 1);
assertTrue("yes still equals 1", "yes" eq 1);
assertTrue("a non-zero decimal string equals true", "1.00" eq true);
assertTrue("a zero decimal string equals false", "0.00" eq false);
// The IEEE spellings are neither numbers nor booleans to CFML.
assertFalse("inf does not equal true", "inf" eq true);
assertFalse("Infinity does not equal true", "Infinity" eq true);

suiteEnd();
</cfscript>
