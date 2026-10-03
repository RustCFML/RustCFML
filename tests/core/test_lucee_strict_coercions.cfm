<cfscript>
suiteBegin("Core: strict boolean conditions, list*At bounds, math domains (Lucee parity)");

// Measured against Lucee 7.1.0.204.
function _r(f) {
	try { var v = f(); return isNull(v) ? "null" : v; } catch (any e) { return "ERR:" & e.type; }
}

// --- conditions: a non-boolean string or a complex value throws ---
assert("if on a non-boolean string throws", _r(function() { if ("abc") return "T"; return "F"; }), "ERR:expression");
assert("if on an empty string throws", _r(function() { if ("") return "T"; return "F"; }), "ERR:expression");
assert("if on a struct throws", _r(function() { if ({}) return "T"; return "F"; }), "ERR:expression");
assert("! on a non-boolean string throws", _r(function() { return !"abc"; }), "ERR:expression");
assert("&& on a non-boolean string throws", _r(function() { return "abc" && true; }), "ERR:expression");
assert("ternary on a non-boolean string throws", _r(function() { return "abc" ? 1 : 2; }), "ERR:expression");
assert("while on a non-boolean string throws", _r(function() { while ("abc") { return "T"; } return "F"; }), "ERR:expression");
assert("iif on a non-boolean string throws", _r(function() { return iif("abc", de("T"), de("F")); }), "ERR:expression");
assert("yesNoFormat on a non-boolean string throws", _r(function() { return yesNoFormat("abc"); }), "ERR:expression");
assert("&& short-circuits before the bad operand", _r(function() { return false && "abc"; }), false);
assert("|| short-circuits before the bad operand", _r(function() { return true || "abc"; }), true);
assert("numeric string 2 is true", _r(function() { if ("2") return "T"; return "F"; }), "T");
assert("numeric string 0.0 is false", _r(function() { if ("0.0") return "T"; return "F"; }), "F");
assert("padded boolean word", _r(function() { if (" true ") return "T"; return "F"; }), "T");
assert("YES is true", _r(function() { if ("YES") return "T"; return "F"; }), "T");
assert("yesNoFormat of an empty string is No", yesNoFormat(""), "No");
assert("trueFalseFormat of 0.0", trueFalseFormat("0.0"), "false");
_caught = "";
try { if ("abc") {} } catch (expression e) { _caught = "caught"; }
assert("the condition error is catchable in the same frame", _caught, "caught");

// --- list*At: out-of-range positions throw ---
assert("listGetAt past the end throws", _r(function() { return listGetAt("a,b", 5); }), "ERR:expression");
assert("listGetAt 0 throws", _r(function() { return listGetAt("a,b", 0); }), "ERR:expression");
assert("listGetAt on an empty list throws", _r(function() { return listGetAt("", 1); }), "ERR:expression");
assert("listGetAt skips empty fields", listGetAt("a,,b", 2), "b");
assert("listGetAt includeEmptyFields", listGetAt("a,,b", 2, ",", true), "");
assert("listGetAt includeEmptyFields last", listGetAt("a,,b", 3, ",", true), "b");
assert("listGetAt multiple delimiters", listGetAt("a;b|c", 3, ";|"), "c");
assert("listSetAt past the end throws", _r(function() { return listSetAt("a,b", 5, "x"); }), "ERR:expression");
assert("listSetAt on an empty list throws", _r(function() { return listSetAt("", 1, "x"); }), "ERR:expression");
assert("listSetAt keeps empty fields", listSetAt("a,,b", 2, "x"), "a,,x");
assert("listSetAt includeEmptyFields", listSetAt("a,,b", 2, "x", ",", true), "a,x,b");
assert("listSetAt keeps edge delimiters", listSetAt(",a,b,", 1, "x"), ",x,b,");
assert("listInsertAt past the end throws", _r(function() { return listInsertAt("a,b", 3, "x"); }), "ERR:expression");
assert("listInsertAt 0 throws", _r(function() { return listInsertAt("a,b", 0, "x"); }), "ERR:expression");
assert("listInsertAt before the first", listInsertAt("a,b", 1, "x"), "x,a,b");
assert("listInsertAt keeps empty fields", listInsertAt("a,,b", 2, "x"), "a,,x,b");
assert("listInsertAt includeEmptyFields", listInsertAt("a,,b", 2, "x", ",", true), "a,x,,b");
assert("listInsertAt uses the first delimiter", listInsertAt("a;b", 2, "x", ";|"), "a;x;b");
assert("listDeleteAt past the end throws", _r(function() { return listDeleteAt("a,b", 3); }), "ERR:expression");
assert("listDeleteAt 0 throws", _r(function() { return listDeleteAt("a,b", 0); }), "ERR:expression");
assert("listDeleteAt of an empty list", listDeleteAt("", 1), "");
assert("listDeleteAt the first element", listDeleteAt("a,,b,c", 1), "b,c");
assert("listDeleteAt a middle element", listDeleteAt("a,,b,c", 2), "a,,c");
assert("listDeleteAt the last element", listDeleteAt("a,,b,c", 3), "a,,b");
assert("listDeleteAt the last after empty fields", listDeleteAt("a,,b", 2), "a");
assert("listDeleteAt keeps edge delimiters", listDeleteAt(",a,b,", 1), ",b,");
assert("listDeleteAt includeEmptyFields", listDeleteAt("a,b,,c,d", 4, ",", true), "a,b,,d");

// --- math: out-of-domain arguments throw ---
assert("sqr of a negative throws", _r(function() { return sqr(-1); }), "ERR:expression");
assert("sqr 0", sqr(0), 0);
assert("log 0 throws", _r(function() { return log(0); }), "ERR:expression");
assert("log of a negative throws", _r(function() { return log(-1.5); }), "ERR:expression");
assert("log10 0 throws", _r(function() { return log10(0); }), "ERR:expression");
assert("asin out of range throws", _r(function() { return asin(2); }), "ERR:expression");
assert("acos out of range throws", _r(function() { return acos(-1.5); }), "ERR:expression");
assert("asin at the edge", round(asin(1) * 1000), 1571);

suiteEnd();
</cfscript>
