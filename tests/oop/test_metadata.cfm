<cfscript>
suiteBegin("Metadata & Missing Method");

// Test onMissingMethod
handler = createObject("component", "oop.MissingMethodHandler");
assert("missing method anyMethod", handler.anyMethod(), "called: anyMethod");
assert("missing method anotherMethod", handler.anotherMethod(), "called: anotherMethod");
assert("missing method fooBar", handler.fooBar(), "called: fooBar");

// getMetadata on Greeter
g = createObject("component", "oop.Greeter").init();
md = getMetadata(g);
assertNotNull("getMetadata returns value", md);
assertTrue("metadata is struct", isStruct(md));
assertTrue("metadata has name", structKeyExists(md, "name"));

// getMetadata() must also expose `fullname` — the fully-qualified component path
// (distinct from the unqualified `name`). Lucee and Adobe both return it.
assertTrue("metadata has fullname", structKeyExists(md, "fullname"));
assertTrue("fullname includes the component name", findNoCase("Greeter", md.fullname ?: "") > 0);

// getMetadata on Dog (has extends)
d = createObject("component", "oop.Dog").init();
dmd = getMetadata(d);
assertNotNull("dog metadata returns value", dmd);
assertTrue("dog metadata has name", structKeyExists(dmd, "name"));

suiteEnd();
</cfscript>
