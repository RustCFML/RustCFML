<cfscript>
suiteBegin("Wheels batch v0.294 engine fixes");

// --- List functions preserve empty elements, index by non-empty (Lucee) ---
assert("ListSetAt preserves leading empties", listSetAt("......wheels00", 1, "Wheels00", "."), "......Wheels00");
assert("ListSetAt 2nd non-empty", listSetAt(",,a,b", 2, "X", ","), ",,a,X");
assert("ListSetAt keeps interior empty", listSetAt("a,,b", 2, "X", ","), "a,,X");
assert("ListInsertAt before nth non-empty", listInsertAt(",,a,b", 2, "X", ","), ",,a,X,b");
assert("ListDeleteAt nth non-empty", listDeleteAt(",,a,b", 2, ","), ",,a");

// --- GetDirectoryFromPath preserves redundant separators ---
assert("GetDirectoryFromPath keeps //", getDirectoryFromPath("/a/b/models//PhotoGallery.cfc"), "/a/b/models//");
assert("GetDirectoryFromPath single", getDirectoryFromPath("/a/b/x.cfc"), "/a/b/");

// --- isValid component / binary ---
// Lucee 7.0/7.1: `component` (and `class`) accept CFC instances only; a Java
// object is an `object`.
assertFalse("isValid component on java obj", isValid("component", createObject("java", "java.lang.StringBuilder")));
assertTrue("isValid object on java obj", isValid("object", createObject("java", "java.lang.StringBuilder")));
assertFalse("isValid component on query", isValid("component", queryNew("id")));
assertTrue("isValid binary", isValid("binary", toBinary(toBase64("hi"))));

// --- isDate rejects bare numerics, accepts date strings ---
assertFalse("isDate(11)", isDate(11));
assertFalse("isDate('11')", isDate("11"));
assertTrue("isDate iso", isDate("2024-01-15"));
assertTrue("isDate no-seconds 12h", isDate("11/01/1975 12:00 AM"));
// Lucee: a bare time-of-day is a valid date (resolves to today at that time).
assertTrue("isDate time 12h", isDate("6:15 PM"));
assertTrue("isDate time 24h", isDate("18:15"));
assertTrue("isValid date time-of-day", isValid("date", "6:15 PM"));

// --- Canonicalize keeps literal + (no form-decode) ---
assert("Canonicalize keeps +", canonicalize("Istok+Web", false, false), "Istok+Web");

// --- isDefined sees query columns ---
qd = queryNew("id,name");
assertTrue("isDefined query col", isDefined("qd.id"));
assertFalse("isDefined missing query col", isDefined("qd.nope"));

// --- package as a bare struct key ---
e = {package = "missingreq", message = "hi"};
assert("package bare struct key", e.package, "missingreq");

// --- cfdirectory action=list on missing dir returns empty query (no throw) ---
cfdirectory(action="list", directory=expandPath("/no/such/dir_zzz/"), name="dq", filter="*.cfc");
assert("cfdirectory missing dir empty", dq.recordCount, 0);

// --- cfsetting requesttimeout round-trips via getPageContext ---
setting requestTimeout=666;
assert("getRequestTimeout ms", getPageContext().getRequestTimeout(), 666000);

// --- implicit set* must not shadow onMissingMethod for unknown properties ---
omm = new compat_engine.fixtures.OnMissingSetter();
assert("set unknown routes to onMissingMethod", omm.setWidget("x"), "OMM:setWidget");
omm.setColor("red"); // declared property -> implicit setter
// Read back through the getter: on Lucee an accessor property is private
// (`omm.color` throws "has no accessible Member"), which RustCFML does not yet
// enforce for a value written by the generated setter.
assert("set known uses implicit setter", omm.getColor(), "red");

// --- closure captures the enclosing function's Function-valued PARAMETER ---
makeMatcher = function(body) {
    return function() { return body(); };
};
mm = makeMatcher(function() { return "captured"; });
assert("closure captures Function param", mm(), "captured");

// --- comparison: non-numeric string vs number falls back to lexical (Lucee) ---
assertTrue("'SQLite' gt 0", "SQLite" > 0);
assertFalse("'SQLite' lt 0", "SQLite" < 0);
assertTrue("'5abc' gt 0 lexical", "5abc" > 0);
assertFalse("'' gt 0", "" > 0);
assertTrue("'5' gt 0 numeric", "5" > 0);
assertFalse("'5' gt 10 numeric", "5" > 10);
assertTrue("'10' gt '9' numeric strings", "10" > "9");

suiteEnd();
</cfscript>
