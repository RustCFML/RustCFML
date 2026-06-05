<cfscript>
suiteBegin("Components");

// Create Greeter with default greeting
g = createObject("component", "oop.Greeter").init();

// Test greet method
assert("greet returns greeting + name", g.greet("World"), "Hello, World!");

// Test getGreeting
assert("getGreeting returns default", g.getGreeting(), "Hello");

// Test setGreeting then getGreeting
g.setGreeting("Hi");
assert("setGreeting updates greeting", g.getGreeting(), "Hi");

// After setGreeting, greet should use new value
assert("greet uses updated greeting", g.greet("World"), "Hi, World!");

// Create with custom greeting via init
g2 = createObject("component", "oop.Greeter").init("Hey");
assert("custom greeting via init", g2.greet("World"), "Hey, World!");
assert("custom getGreeting", g2.getGreeting(), "Hey");

// Verify this scope property exists
assertTrue("greeting property exists on this", structKeyExists(g2, "greeting"));

// Verify the greeting value via this scope
assert("this.greeting value", g2.greeting, "Hey");

// Test isObject
assertTrue("isObject on component", isObject(g));

// Test isStruct (components are struct-like)
assertTrue("isStruct on component", isStruct(g));

// Verify method exists via structKeyExists
assertTrue("greet method exists", structKeyExists(g, "greet"));
assertTrue("getGreeting method exists", structKeyExists(g, "getGreeting"));

// Single-argument createObject() shorthand.
// createObject("path") is shorthand for createObject("component","path") and must
// return an instantiated component, identical to the two-arg form used above.
gShort = createObject("oop.Greeter");
assertTrue("single-arg createObject returns an object", isObject(gShort));
assert("single-arg createObject instance is usable", gShort.init().greet("World"), "Hello, World!");

suiteEnd();
</cfscript>
