<cfscript>
suiteBegin("Argument Bounds");

// Every call here used to PANIC, ABORT (allocation failure) or HANG the engine
// on an out-of-range argument that any caller can pass from user input. Each
// must now throw a catchable CFML error naming the argument, or — where Lucee
// accepts the value with a defined result — produce that result.
//
// Growth caps (arrays, query rows, padded strings) are RustCFML's own: Lucee
// has no cap and relies on a JVM OutOfMemoryError. A write that would grow a
// container by more than 100,000,000 elements in one call is refused here.
// PBKDF iterations are capped at 10,000,000 and the key size at 4096 bits.

// ---- arrayMid (Lucee ArrayMid: start >= 1; count -1 = rest; count < -1 is an error)
a = [1, 2, 3, 4, 5];
assert("arrayMid count -1 means the rest", arrayToList(arrayMid(a, 2, -1)), "2,3,4,5");
assert("arrayMid count 0 is empty", arrayLen(arrayMid(a, 2, 0)), 0);
assert("arrayMid valid span unchanged", arrayToList(arrayMid(a, 2, 3)), "2,3,4");
assertThrows("arrayMid start -1", function() { arrayMid([1, 2, 3], -1, 2); });
assertThrows("arrayMid start 0", function() { arrayMid([1, 2, 3], 0, 2); });
assertThrows("arrayMid count -5", function() { arrayMid([1, 2, 3], 2, -5); });
try { arrayMid([1, 2, 3], -1, 2); } catch (any e) { assertTrue("arrayMid error names start", findNoCase("start", e.message) > 0); }
try { arrayMid([1, 2, 3], 2, -5); } catch (any e) { assertTrue("arrayMid error names count", findNoCase("count", e.message) > 0); }

// ---- arraySplice (Lucee ArraySplice: no errors — index < 1 counts from the
// end, length -1 = rest, length < -1 = nothing removed)
a = [1, 2, 3, 4, 5];
r = arraySplice(a, 2, -1);
assert("arraySplice length -1 removes the rest", arrayToList(r), "2,3,4,5");
assert("arraySplice length -1 leaves the head", arrayToList(a), "1");
a = [1, 2, 3, 4, 5];
r = arraySplice(a, 2, -7);
assert("arraySplice length < -1 removes nothing", arrayLen(r), 0);
assert("arraySplice length < -1 leaves the array", arrayToList(a), "1,2,3,4,5");
a = [1, 2, 3, 4, 5];
r = arraySplice(a, -1, 1);
assert("arraySplice index -1 is the last element", arrayToList(r), "5");
assert("arraySplice index -1 leaves the others", arrayToList(a), "1,2,3,4");
a = [1, 2, 3, 4, 5];
r = arraySplice(a, 2, 2, ["x"]);
assert("arraySplice valid removed", arrayToList(r), "2,3");
assert("arraySplice valid result", arrayToList(a), "1,x,4,5");

// ---- querySlice (Lucee QuerySlice: length <= 0 counts back from the end;
// offset+length past the end is an error; empty query is an error)
q = queryNew("a", "integer", [[1], [2], [3], [4], [5]]);
assert("querySlice length -1 stops one short of the end", valueList(querySlice(q, 2, -1).a), "2,3,4");
assert("querySlice valid unchanged", valueList(querySlice(q, 2, 2).a), "2,3");
assert("querySlice offset only", valueList(querySlice(q, 4).a), "4,5");
assert("querySlice offset -2 counts from the end", valueList(querySlice(q, -2, 2).a), "3,4");
assertThrows("querySlice offset past the end", function() { querySlice(q, 9, 1); });
assertThrows("querySlice offset+length past the end", function() { querySlice(q, 4, 5); });
assertThrows("querySlice empty query", function() { querySlice(queryNew("a"), 1, 1); });

// ---- lJustify / rJustify / cJustify (Lucee: length < 1 is an error)
assert("lJustify valid", lJustify("ab", 5), "ab   ");
assert("rJustify valid", rJustify("ab", 5), "   ab");
assert("cJustify valid", cJustify("ab", 6), "  ab  ");
assert("cJustify odd padding goes right", cJustify("a", 4), " a  ");
assert("lJustify shorter length returns the string", lJustify("abc", 2), "abc");
assert("cJustify counts characters, not bytes", cJustify("é", 5), "  é  ");
assert("lJustify counts characters, not bytes", len(lJustify("é", 5)), 5);
assertThrows("lJustify length 0", function() { lJustify("a", 0); });
assertThrows("rJustify length 0", function() { rJustify("a", 0); });
assertThrows("cJustify length -1", function() { cJustify("a", -1); });
assertThrows("cJustify length 0", function() { cJustify("a", 0); });
if (isRustCFML()) {
	// Lucee would try to build the 4 GB string (OutOfMemoryError); the cap is ours.
	assertThrows("lJustify length 4e9", function() { lJustify("a", 4000000000); });
	assertThrows("rJustify length 4e9", function() { rJustify("a", 4000000000); });
	assertThrows("cJustify length 4e9", function() { cJustify("a", 4000000000); });
	assert("lJustify 70000 (above the u16 formatter limit) works", len(lJustify("a", 70000)), 70000);
	assert("rJustify 70000 works", len(rJustify("a", 70000)), 70000);
	assert("cJustify 70000 works", len(cJustify("a", 70000)), 70000);
}

// ---- numberFormat with an absurd decimal mask
assert("numberFormat 2 decimals unchanged", numberFormat(1, "9.99"), "1.00");
assert("numberFormat 20 decimals works", numberFormat(1, "9." & repeatString("9", 20)), "1." & repeatString("0", 20));
if (isRustCFML()) {
	assertThrows("numberFormat mask with 100000 decimal places", function() { numberFormat(1, "9." & repeatString("9", 100000)); });
}

// ---- inputBaseN (Lucee: radix must be between 2 and 36)
assert("inputBaseN valid", inputBaseN("ff", 16), 255);
assertThrows("inputBaseN radix 1", function() { inputBaseN("111", 1); });
assertThrows("inputBaseN radix 0", function() { inputBaseN("111", 0); });
assertThrows("inputBaseN radix 99", function() { inputBaseN("111", 99); });
assertThrows("inputBaseN radix -2", function() { inputBaseN("111", -2); });
try { inputBaseN("111", 99); } catch (any e) { assertTrue("inputBaseN error names radix", findNoCase("radix", e.message) > 0); }

// ---- bitSHLN / bitSHRN (Lucee: 64-bit, count < 0 is an error, a count of 64+
// is masked to 0..63 the way Java masks a long shift; bitSHRN is UNSIGNED)
assert("bitSHLN valid", bitSHLN(1, 3), 8);
assert("bitSHRN valid", bitSHRN(8, 3), 1);
assert("bitSHLN count 64 wraps to count 0 (Java long shift)", bitSHLN(1, 64), 1);
assert("bitSHRN count 64 wraps to count 0 (Java long shift)", bitSHRN(1, 64), 1);
assert("bitSHRN is a logical shift", bitSHRN(-1, 1), 9223372036854775807);
assertThrows("bitSHLN count -1", function() { bitSHLN(1, -1); });
assertThrows("bitSHRN count -1", function() { bitSHRN(1, -1); });

// ---- bitMaskRead / bitMaskSet / bitMaskClear (Lucee: 32-bit; start and
// length must be between 0 and 31)
assert("bitMaskRead valid", bitMaskRead(255, 4, 4), 15);
assert("bitMaskSet valid", bitMaskSet(0, 15, 4, 4), 240);
assert("bitMaskClear valid", bitMaskClear(255, 0, 4), 240);
assert("bitMaskRead is 32-bit", bitMaskRead(-1, 0, 31), 2147483647);
assertThrows("bitMaskRead start -1", function() { bitMaskRead(1, -1, 4); });
assertThrows("bitMaskRead length 99", function() { bitMaskRead(1, 0, 99); });
assertThrows("bitMaskSet start -1", function() { bitMaskSet(1, 1, -1, 4); });
assertThrows("bitMaskSet length 99", function() { bitMaskSet(1, 1, 0, 99); });
assertThrows("bitMaskClear start -1", function() { bitMaskClear(1, -1, 4); });
assertThrows("bitMaskClear length 99", function() { bitMaskClear(1, 0, 99); });
assertThrows("bitMaskRead number beyond 32 bits", function() { bitMaskRead(4294967296, 0, 4); });
try { bitMaskRead(1, -1, 4); } catch (any e) { assertTrue("bitMask error names the range", find("0 and 31", e.message) > 0); }

// ---- randRange (Lucee swaps a reversed range; a span wider than the integer
// type is refused here rather than overflowing)
r = randRange(10, 1);
assertTrue("randRange reversed range swaps", r >= 1 && r <= 10);
// Lucee computes the span in double arithmetic and returns a value; RustCFML
// refuses a span that does not fit in 64 bits.
if (isRustCFML()) {
    assertThrows("randRange span wider than 64 bits", function() { randRange(-9223372036854775808, 9223372036854775807); });
}

// ---- array index writes far beyond the length (growth cap, RustCFML only:
// Lucee fills with nulls up to any index and relies on the JVM heap limit)
a = [];
a[5] = 1;
assert("array write past the end fills with nulls", arrayLen(a), 5);
if (isRustCFML()) {
	assertThrows("a[9223372036854775807] = 1", function() { var b = []; b[9223372036854775807] = 1; });
	assertThrows("a[4000000000] = 1", function() { var b = []; b[4000000000] = 1; });
	try { b = []; b[4000000000] = 1; } catch (any e) { assertTrue("array growth error names the index", find("4000000000", e.message) > 0); }
}

// ---- arraySet / arrayResize / arrayRange / queryAddRow
a = [1];
arraySet(a, 1, 3, 0);
assert("arraySet valid grows", arrayToList(a), "0,0,0");
assertThrows("arraySet end before start", function() { var b = [1, 2, 3]; arraySet(b, 3, 1, 0); });
a = [1];
arrayResize(a, 3);
assert("arrayResize valid", arrayLen(a), 3);
// arrayRange is a BoxLang/RustCFML function; Lucee 7 has no such BIF.
if (isRustCFML()) {
    assert("arrayRange valid", arrayToList(arrayRange(1, 5)), "1,2,3,4,5");
}
q = queryNew("a");
assert("queryAddRow 2 returns the row count", queryAddRow(q, 2), 2);
assert("queryAddRow 0 adds nothing", queryAddRow(q, 0), 2);
// Lucee's addRow(-1) is a silent no-op; RustCFML refuses a negative count.
if (isRustCFML()) {
assertThrows("queryAddRow -1", function() { var q2 = queryNew("a"); queryAddRow(q2, -1); });
}
if (isRustCFML()) {
	assertThrows("arraySet end 4e9", function() { var b = [1]; arraySet(b, 1, 4000000000, 0); });
	assertThrows("arrayResize 4e9", function() { var b = [1]; arrayResize(b, 4000000000); });
	assertThrows("arrayRange 1..4e9", function() { arrayRange(1, 4000000000); });
	assertThrows("queryAddRow 4e9", function() { var q2 = queryNew("a"); queryAddRow(q2, 4000000000); });
}

// ---- generatePBKDFKey / PBEKeySpec (Java PBEKeySpec refuses iterationCount
// and keyLength below 1; the upper caps are RustCFML's)
assert("generatePBKDFKey valid", generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", 1000, 128), "rIuV2z7QIBdq/0DBQAOWPw==");
assertThrows("generatePBKDFKey keySize -8", function() { generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", 1, -8); });
assertThrows("generatePBKDFKey iterations -1", function() { generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", -1, 128); });
assertThrows("generatePBKDFKey iterations 0", function() { generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", 0, 128); });
assertThrows("PBEKeySpec keyLength -8", function() {
	var spec = createObject("java", "javax.crypto.spec.PBEKeySpec").init("p".toCharArray(), charsetDecode("s", "utf-8"), 1, -8);
	createObject("java", "javax.crypto.SecretKeyFactory").getInstance("PBKDF2WithHmacSHA1").generateSecret(spec);
});
assertThrows("PBEKeySpec iterationCount -1", function() {
	var spec = createObject("java", "javax.crypto.spec.PBEKeySpec").init("p".toCharArray(), charsetDecode("s", "utf-8"), -1, 128);
	createObject("java", "javax.crypto.SecretKeyFactory").getInstance("PBKDF2WithHmacSHA1").generateSecret(spec);
});
if (isRustCFML()) {
	assertThrows("generatePBKDFKey iterations 2e9", function() { generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", 2000000000, 128); });
	assertThrows("generatePBKDFKey keySize 4e9", function() { generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", 1, 4000000000); });
	assertThrows("generatePBKDFKey keySize 8192", function() { generatePBKDFKey("PBKDF2WithHmacSHA1", "p", "s", 1, 8192); });
}

// ---- whole-file reads of a non-regular file (RustCFML refuses a device or
// pipe up front; Lucee would read /dev/zero until the heap is exhausted)
if (isRustCFML() && fileExists("/etc/hosts") && !findNoCase("windows", server.os.name)) {
	assertThrows("fileRead /dev/zero", function() { fileRead("/dev/zero"); });
	assertThrows("fileRead /dev/zero with charset", function() { fileRead("/dev/zero", "utf-8"); });
	assertThrows("fileReadBinary /dev/zero", function() { fileReadBinary("/dev/zero"); });
	assertThrows("fileRead /dev/random", function() { fileRead("/dev/random"); });
	assertThrows("include /dev/zero", function() { include "/dev/zero"; });
	assertThrows("fileCopy /dev/zero", function() { fileCopy("/dev/zero", getTempDirectory() & "/argbounds_zero_copy"); });
	try { fileRead("/dev/zero"); } catch (any e) { assertTrue("non-regular file error says so", findNoCase("not a regular file", e.message) > 0); }
	assertTrue("fileRead of a regular file still works", len(fileRead("/etc/hosts")) > 0);
}

// ---- createTime: an hour too large for a Duration panicked inside chrono.
// Lucee answers the same epoch-0 time it gives for any other invalid field.
invalidTime = timeFormat(createTime(1, 99999999999, 0), "HH:mm:ss");
assert("createTime huge hour is the invalid-field time", timeFormat(createTime(9223372036854775807), "HH:mm:ss"), invalidTime);
assert("createTime hour 25 still rolls over", timeFormat(createTime(25, 0, 0), "HH:mm:ss"), "01:00:00");

// ---- Integer arguments coerce as Lucee's Caster does: trimmed, a fraction
// truncates, a boolean literal is 1/0, anything else throws. A non-integer
// string used to read as 0 (mid started at the first character).
assert("mid start ' 2'", mid("abcdef", " 2", 3), "bcd");
assert("mid start '2.9' truncates", mid("abcdef", "2.9", 3), "bcd");
assert("mid start '2e0'", mid("abcdef", "2e0", 3), "bcd");
assert("left count ' 2'", left("abcdef", " 2"), "ab");
assert("left count 'yes' is 1", left("abcdef", "yes"), "a");
assert("repeatString count ' 2'", repeatString("ab", " 2"), "abab");
assert("listGetAt position ' 2'", listGetAt("a,b,c", " 2"), "b");
assertThrows("mid start 'abc' throws", function() { mid("abcdef", "abc", 3); });
assertThrows("mid start '' throws", function() { mid("abcdef", "", 3); });
assertThrows("left count '0x2' throws", function() { left("abcdef", "0x2"); });
assertThrows("left count [] throws", function() { left("abcdef", []); });
assert("mid count -1 is the rest", mid("abcdef", 2, -1), "bcdef");
assertThrows("mid count below -1 throws", function() { mid("abcdef", 2, -2); });

// ---- formatBaseN takes a Java int: 4294967296 wrapped to 0 and printed "0".
// Radix 2 and 16 print a negative as its unsigned two's complement.
assertThrows("formatBaseN past Integer.MAX_VALUE throws", function() { formatBaseN(4294967296, 16); });
assert("formatBaseN -1 radix 16", formatBaseN(-1, 16), "ffffffff");
assert("formatBaseN -255 radix 16", formatBaseN(-255, 16), "ffffff01");
assert("formatBaseN -1 radix 2", formatBaseN(-1, 2), "11111111111111111111111111111111");
assert("formatBaseN -1 radix 10 stays signed", formatBaseN(-1, 10), "-1");
assert("formatBaseN Integer.MIN_VALUE radix 16", formatBaseN(-2147483648, 16), "80000000");
assertThrows("formatBaseN radix 1 throws", function() { formatBaseN(10, 1); });

// ---- inputBaseN: an unparseable string throws, as Java's parseLong does.
assert("inputBaseN trims", inputBaseN(" ff", 16), 255);
assertThrows("inputBaseN 'zz' radix 10 throws", function() { inputBaseN("zz", 10); });
assertThrows("inputBaseN '' throws", function() { inputBaseN("", 16); });

// ---- Image sizes: a negative or huge size wrapped through `as u32`.
function br_img() { return imageNew("", 20, 10, "rgb", "white"); }
i = br_img(); imageCrop(i, -10, 0, 5, 5);
assert("imageCrop clamps a negative origin", imageGetWidth(i) & "x" & imageGetHeight(i), "5x5");
i = br_img(); imageCrop(i, 15, 0, 10, 5);
assert("imageCrop clips the size to the image", imageGetWidth(i) & "x" & imageGetHeight(i), "5x5");
assert("image width property", i.width, 5);
assertThrows("imageCrop past the edge throws", function() { var j = br_img(); imageCrop(j, 30, 0, 5, 5); });
assertThrows("imageCrop negative width throws", function() { var j = br_img(); imageCrop(j, 0, 0, -5, 5); });
assertThrows("imageResize to 1e12 throws", function() { var j = br_img(); imageResize(j, 1e12, 5); });
assertThrows("imageResize to 100000x100000 throws", function() { var j = br_img(); imageResize(j, 100000, 100000); });

suiteEnd();
</cfscript>
