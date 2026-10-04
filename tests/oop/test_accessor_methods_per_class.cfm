<cfscript>
// Generated accessors are attached once per class with the declared methods
// (they used to be rebuilt by the pseudo-constructor on every `new`). Pins the
// observable contract on both engines: the accessors are in `this` and
// `variables`, a method the class declares itself wins over its generated twin,
// setters chain, and metadata lists them.
suiteBegin("Generated accessors are per-class methods");

apA = new oop.AccessorPerClassProbe();
apP = apA.probe();
assertTrue("getter visible in variables", apP.varHasGetter);
assertTrue("setter visible in variables", apP.varHasSetter);
assertTrue("getter visible in this", apP.thisHasGetter);
assert("bare getter call inside the class", apP.bareGetter, "L");
assert("a declared getter wins over the generated one", apP.explicitWins, "explicit");
assert("the declared getter also wins via variables", apP.viaVariables, "explicit");
apA.setLabel("Z");
assert("setter then getter", apA.getLabel(), "Z");
assert("setters return this", apA.setLabel("Q").getLabel(), "Q");

apNames = [];
for (apF in getMetaData(apA).functions) arrayAppend(apNames, apF.name);
arraySort(apNames, "textnocase");
assert("metadata lists accessors once each", arrayToList(apNames), "getCustom,getLabel,probe,setCustom,setLabel");

// Instances are independent.
apB = new oop.AccessorPerClassProbe();
apB.setLabel("B");
assert("a second instance keeps its own value", apA.getLabel(), "Q");
assert("and the second has its own", apB.getLabel(), "B");

suiteEnd();
</cfscript>
