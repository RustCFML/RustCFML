<cfscript>
suiteBegin("Integer Overflow Promotion");

// Lucee computes in doubles, so arithmetic past the 64-bit integer range just
// loses precision. RustCFML keeps 64-bit integers and used to PANIC (debug) or
// WRAP to a negative (release) when `+`, `++`, `+=`, `*=`, a loop increment or
// incrementValue() crossed i64::MAX. `*` and `-` already promoted to a double
// on overflow; these now do the same, so every form below agrees with Lucee.
big = 9223372036854775807;

// A literal past i64 is a double (Lucee: every literal is); it used to lex as 0.
assertTrue("integer literal past i64 is not 0", 9223372036854775808 > 1);
assertTrue("negative literal past i64 is not 0", -9223372036854775809 < -1);

r = big + 1;
assertTrue("i64::MAX + 1 is a number", isNumeric(r));
assertTrue("i64::MAX + 1 promotes, does not wrap negative", r > 0);
assert("i64::MAX + 1 equals the double", r, 9223372036854775808);

i = big; i++;
assertTrue("i++ past i64::MAX promotes", i > 0);
assert("i++ past i64::MAX value", i, 9223372036854775808);

i = big; i += 1;
assertTrue("i += 1 past i64::MAX promotes", i > 0);

i = big; i *= 2;
assertTrue("i *= 2 past i64::MAX promotes", i > 0);
assert("i *= 2 value", i, 18446744073709551616);

i = -9223372036854775808; i--;
assertTrue("i-- past i64::MIN promotes", i < 0);
i = -9223372036854775808; i -= 1;
assertTrue("i -= 1 past i64::MIN promotes", i < 0);

assertTrue("incrementValue(i64::MAX) promotes", incrementValue(big) > 0);
assertTrue("decrementValue(i64::MIN) promotes", decrementValue(-9223372036854775808) < 0);

// A for loop whose increment crosses i64::MAX terminates instead of dying.
n = 0;
for (i = 9223372036854775806; i <= big; i++) { n++; if (n > 5) break; }
assertTrue("for loop crossing i64::MAX terminates", n >= 1 && n <= 6);

// cfloop from/to/step near i64::MAX (the step add used to panic).
n = 0;
cfloop(index = "j", from = 9223372036854775806, to = big, step = 2) { n++; if (n > 5) break; }
assertTrue("cfloop crossing i64::MAX terminates", n >= 1 && n <= 6);

// Normal integer arithmetic is untouched.
assert("plain add", 2 + 3, 5);
x = 5; x++; x += 10; x *= 2;
assert("plain fused ops", x, 32);

// java.time shims: Java throws ArithmeticException("long overflow") where the
// millisecond conversion overflows.
assertThrows("Instant.ofEpochSecond(i64::MAX)", function() { createObject("java", "java.time.Instant").ofEpochSecond(9223372036854775807); });
assertThrows("Instant.plusSeconds(i64::MAX)", function() { createObject("java", "java.time.Instant").now().plusSeconds(9223372036854775807); });
assertThrows("Duration.ofDays(i64::MAX)", function() { createObject("java", "java.time.Duration").ofDays(9223372036854775807); });
assert("Instant.ofEpochSecond valid", createObject("java", "java.time.Instant").ofEpochSecond(10).toEpochMilli(), 10000);
assert("Duration.ofDays valid", createObject("java", "java.time.Duration").ofDays(1).toMillis(), 86400000);

// `i64::MIN mod -1` panicked the request (Rust's `%` overflows there) and
// negating i64::MIN wrapped. Lucee computes both in doubles.
lo = -9223372036854775807 - 1;
minusOne = -1;
assert("i64::MIN mod -1 is 0", lo mod minusOne, 0);
assert("i64::MIN % -1 is 0", lo % minusOne, 0);
r = lo; r %= minusOne;
assert("i64::MIN %= -1 is 0", r, 0);
assertTrue("-(i64::MIN) is positive, not wrapped", -lo > 0);

suiteEnd();
</cfscript>
