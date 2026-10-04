<cfscript>
// A class whose body only declares (properties with literal defaults, methods;
// no pseudo-constructor statements) produces equal data on every construction,
// so the engine may build later instances from a copy of the first one this
// request. These pin everything such a copy must preserve. They pass on Lucee
// (no such fast path there), so they hold the engine to plain CFML semantics.
suiteBegin("Construction of declaration-only classes");

dpA = new oop.DeclProtoSvc("a");
dpB = new oop.DeclProtoSvc("b");
dpC = createObject("component", "oop.DeclProtoSvc").init("c");

// init ran for each instance, and each keeps its own variables.
assert("init per instance (a)", dpA.getName(), "a");
assert("init per instance (b)", dpB.getName(), "b");
assert("init per instance (c)", dpC.getName(), "c");
assert("init ran once on a", dpA.getInits(), 1);
assert("init ran once on b", dpB.getInits(), 1);

// State is per instance.
dpA.getOpts().k = 1;
arrayAppend(dpA.getTags(), "x");
assertFalse("struct built in init not shared", structKeyExists(dpB.getOpts(), "k"));
assert("array built in init not shared", arrayLen(dpB.getTags()), 0);
assert("scalar default", dpB.getLabel(), "L");
assert("numeric default", dpB.getCount(), 0);
assert("inherited default", dpB.getBaseLabel(), "B");
dpB.setLabel("Z");
dpB.setCount(5);
dpB.setBaseLabel("Q");
assert("setter affects only that instance", dpA.getLabel(), "L");
assert("numeric setter affects only that instance", dpA.getCount(), 0);
assert("inherited setter affects only that instance", dpA.getBaseLabel(), "B");
dpE = new oop.DeclProtoSvc("e");
assert("a later instance starts from the defaults", dpE.getLabel() & dpE.getCount() & dpE.getBaseLabel(), "L0B");

// Identity, type and metadata.
assertFalse("distinct objects", dpA === dpB);
assertTrue("isInstanceOf class", isInstanceOf(dpA, "oop.DeclProtoSvc"));
assertTrue("isInstanceOf parent", isInstanceOf(dpB, "oop.DeclProtoBase"));
assertTrue("isInstanceOf interface", isInstanceOf(dpC, "oop.DeclProtoIface"));
assert("metadata name", getMetaData(dpB).name, getMetaData(dpA).name);
assert("same public keys", listSort(structKeyList(dpB), "textnocase"), listSort(structKeyList(dpA), "textnocase"));

// Inheritance, super, static and variables.this.
assert("super dispatch", dpB.ping(), "child>base-ping");
assert("inherited method", dpC.baseOnly(), "base-only");
assert("static shared across instances", dpC.staticCount(), dpA.staticCount());
assertTrue("static counts every init", dpC.staticCount() >= 3);
assert("variables.this is the instance", dpB.selfViaVariablesThis(), "b");

// A non-literal default is evaluated per instance (not prototyped).
dpD1 = new oop.DeclProtoDynamic();
dpD2 = new oop.DeclProtoDynamic();
assertFalse("dynamic default differs per instance", dpD1.getStamp() == dpD2.getStamp());

// A class without init keeps working, repeatedly.
dpN1 = createObject("component", "oop.DeclProtoNoInit");
dpN2 = createObject("component", "oop.DeclProtoNoInit");
dpN1.setColor("blue");
assert("no-init class: independent", dpN2.getColor(), "red");
dpN3 = new oop.DeclProtoNoInit(color = "green");
assert("implicit accessor constructor", dpN3.getColor(), "green");

// Many in a loop stay independent.
dpList = [];
for (dpI = 1; dpI <= 50; dpI++) {
    dpO = new oop.DeclProtoSvc("n" & dpI);
    dpO.getOpts().i = dpI;
    arrayAppend(dpList, dpO);
}
assert("loop instance 1 kept its own state", dpList[1].getOpts().i, 1);
assert("loop instance 50 kept its own state", dpList[50].getOpts().i, 50);
assert("loop instance 25 name", dpList[25].getName(), "n25");

suiteEnd();
</cfscript>
