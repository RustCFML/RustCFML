<cfscript>
suiteBegin("Stdlib: positional string BIFs on long strings + StringBuilder/ArraySlice (GH ##460, ##461)");

// Long enough to take the memoised path (>= 256 bytes); mixed ASCII/non-ASCII
// so character and byte positions differ.
base = repeatString("ab", 200) & "é" & repeatString("cd", 200) & "ü'xyz'" & repeatString("e", 300);
assert("len counts characters", len(base), 400 + 1 + 400 + 6 + 300);
p1 = find("'", base);
assert("find past a multibyte char", p1, 803);
assert("mid at the found position", mid(base, p1, 1), "'");
p2 = find("'", base, p1 + 1);
assert("forward find from the previous hit", p2, 807);
assert("mid between the two hits", mid(base, p1 + 1, p2 - p1 - 1), "xyz");
// Jumping backwards must still be right.
assert("find after a backwards jump", find("é", base, 1), 401);
assert("mid after a backwards jump", mid(base, 401, 3), "écd");
assert("mid back at the start", mid(base, 1, 4), "abab");
assert("left across the multibyte char", right(left(base, 402), 3), "béc");
assert("right", right(base, 3), "eee");
assert("find start past the end", find("e", base, len(base) + 2), 0);
assert("find start at the last char", find("e", base, len(base)), len(base));
assert("mid start past the end", mid(base, len(base) + 5, 2), "");
assert("findNoCase on a non-ASCII haystack", findNoCase("XYZ", base), 804);
assert("findNoCase from a start", findNoCase("Ü", base, 400), 802);
ascii = repeatString("abcdQ", 100);
assert("findNoCase ASCII forward", findNoCase("q", ascii, 6), 10);
assert("findNoCase ASCII miss", findNoCase("qq", ascii), 0);
assert("findNoCase ASCII past the end", findNoCase("q", ascii, 1000), 0);

// A scan alternating find/mid over the same string.
s = repeatString("ébcd'", 300);
count = 0; i = 1;
while (true) {
	q = find("'", s, i);
	if (q == 0) break;
	if (mid(s, q, 1) == "'") count++;
	i = q + 1;
}
assert("alternating find/mid scan finds every quote", count, 300);

// A string modified in place gets a fresh identity — positions stay right.
t = repeatString("x", 300);
assert("find before append", find("y", t), 0);
t &= "y";
assert("find after in-place append", find("y", t), 301);

// StringBuilder: append is in place, toString() is a snapshot.
sb = createObject("java", "java.lang.StringBuilder").init("a");
sb.append("b");
snap = sb.toString();
sb.append("c");
assert("toString snapshot is not changed by a later append", snap, "ab");
assert("builder has every append", sb.toString(), "abc");
sb.append("42");
assert("append a second string", sb.toString(), "abc42");
function addTo(b) { b.append("!"); }
addTo(sb);
assert("append through a function argument", sb.toString(), "abc42!");

// arraySlice bounds, measured against Lucee 7.1.0.204.
arr = [1, 2, 3, 4, 5];
assert("arraySlice middle", arrayToList(arraySlice(arr, 2, 3)), "2,3,4");
assert("arraySlice to end", arrayToList(arraySlice(arr, 4)), "4,5");
assert("arraySlice exactly to the end", arrayToList(arraySlice(arr, 4, 2)), "4,5");
assertThrows("arraySlice length past the end throws", function() { arraySlice([1, 2, 3, 4, 5], 4, 3); });
assertThrows("arraySlice offset past the end throws", function() { arraySlice([1, 2, 3, 4, 5], 6); });
assertThrows("arraySlice of an empty array throws", function() { arraySlice([], 1); });
assert("arraySlice offset 0 is the last element", arrayToList(arraySlice(arr, 0)), "5");
assert("arraySlice negative offset counts back from len", arrayToList(arraySlice(arr, -2)), "3,4,5");
assert("arraySlice negative offset with length", arrayToList(arraySlice(arr, -2, 1)), "3");
assert("arraySlice negative offset past the start wraps", arrayToList(arraySlice(arr, -9)), "1,2,3,4,5");
assert("arraySlice length 0 is to the end", arrayToList(arraySlice(arr, 2, 0)), "2,3,4,5");
assert("arraySlice negative length stops before the end", arrayToList(arraySlice(arr, 2, -1)), "2,3,4");
assert("arraySlice negative length to before offset", arrayLen(arraySlice(arr, 2, -4)), 0);
assert("arraySlice does not alter the source", arrayToList(arr), "1,2,3,4,5");

suiteEnd();
</cfscript>
