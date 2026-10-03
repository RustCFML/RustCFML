<cfscript>
suiteBegin("CFC methods declared by an included template (GH ##463, ##464)");

o = createObject("component", "oop.IncludeMethods");
assert("included public method is a method of the instance", o.fromInclude(), "included:inline:s3");
assert("included public method can call an included private one", o.callsPrivate(), "private");
assertThrows("included private method is not callable from outside", function() { o.hiddenFromInclude(); });
assertThrows("an included method is not a bare page function (Lucee parity)", function() { fromInclude(); });

// A second instance takes the memoised path: same behaviour, independent state.
o2 = createObject("component", "oop.IncludeMethods");
assert("second instance has the included method", o2.fromInclude(), "included:inline:s3");
o2.pubData = 2;
assert("instances do not share data", o.pubData, 1);

// structKeyExists on a component is a direct probe, not an enumeration.
assertTrue("structKeyExists sees a public included method", structKeyExists(o, "fromInclude"));
assertTrue("structKeyExists is case-insensitive", structKeyExists(o, "FROMINCLUDE"));
assertTrue("structKeyExists sees an inline public method", structKeyExists(o, "inlineOne"));
assertTrue("structKeyExists sees public data", structKeyExists(o, "pubData"));
assertFalse("structKeyExists hides a private method", structKeyExists(o, "hiddenFromInclude"));
assertFalse("structKeyExists hides a private variable", structKeyExists(o, "secret"));
assertFalse("structKeyExists reports a null public member absent", structKeyExists(o, "nullData"));
assertFalse("structKeyExists: missing key", structKeyExists(o, "nope"));
assertFalse("structKeyExists: engine-internal names are not keys", structKeyExists(o, "__variables"));

suiteEnd();
</cfscript>
