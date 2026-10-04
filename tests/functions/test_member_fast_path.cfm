<cfscript>
suiteBegin("array/struct member calls keep their member contracts");
// Expected values verified on Lucee 7.1.0.204.
a = [1];
r = a.append(2);
assertTrue("append returns the receiver itself", r === a);
assert("append mutates in place", arrayLen(a), 2);
assert("a.len()", a.len(), 2);
assertTrue("[].isEmpty()", [].isEmpty());
assertFalse("a.isEmpty()", a.isEmpty());
m = [[1], [2]];
m[1].append(9);
assert("append on an element expression", serializeJSON(m), "[[1,9],[2]]");
s = { b = 1, n = javaCast("null", "") };
assertTrue("keyExists is case-insensitive", s.keyExists("B"));
assertFalse("keyExists on a missing key", s.keyExists("zz"));
assertFalse("keyExists on a null-valued key", s.keyExists("n"));
assert("s.len() counts a null-valued key", s.len(), 2);
assertTrue("{}.isEmpty()", {}.isEmpty());
u = { append = function(x) { return "udf:" & x; }, len = function() { return "udfLen"; } };
assert("a struct's own function named append is called", u.append(5), "udf:5");
assert("a struct's own function named len is called", u.len(), "udfLen");
function viaArgs(required array arr) { arguments.arr.append(3); return arguments.arr.len(); }
assert("member calls through arguments", viaArgs([1, 2]), 3);
assert("append with merge flag still takes the full path", arrayLen([1].append([2, 3], true)), 3);
suiteEnd();
</cfscript>
