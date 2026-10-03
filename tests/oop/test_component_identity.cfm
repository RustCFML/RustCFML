<cfscript>
suiteBegin("OOP: component identity matches Lucee (GH ##452)");

// Measured against a freshly started Lucee 7.1.0.204. Fixtures are used by
// this file only: Lucee reuses a component's cached name for the rest of a run,
// so a fixture loaded elsewhere first would make this file order-dependent.

// 1. A component found RELATIVE to the calling page is named by its full
//    webroot-relative path, not by the name as written.
rel = createObject("component", "ident452.Plain452");
assert("relative createObject: full dotted name",
	getMetadata(rel).name, "tests.oop.ident452.Plain452");
relNew = new ident452.sub.Deep452();
assert("relative new: full dotted name",
	getMetadata(relNew).name, "tests.oop.ident452.sub.Deep452");
assert("isInstanceOf: full name", isInstanceOf(rel, "tests.oop.ident452.Plain452"), true);
assert("isInstanceOf: name as written", isInstanceOf(rel, "ident452.Plain452"), true);
assert("isInstanceOf: bare name", isInstanceOf(rel, "Plain452"), true);

// A relative name inside a component takes that component's package.
child = new ident452.Child452();
assert("relative new inside a component: sibling",
	getMetadata(child.makeSibling()).name, "tests.oop.ident452.Sib452");
assert("relative new inside a component: sub-package",
	getMetadata(child.makeNested()).name, "tests.oop.ident452.sub.Deep452");

// A missing member reports the component by the name it was created under.
msg = "";
try { x = rel.nope; } catch (any e) { msg = e.message; }
assert("missing member message",
	msg, "Component [ident452.Plain452] has no accessible Member with name [NOPE]");
msg = "";
try { x = rel.priv; } catch (any e) { msg = e.message; }
assert("private member is not readable from outside",
	msg, "Component [ident452.Plain452] has no accessible Member with name [PRIV]");
assert("public this member is readable", rel.pub, 1);

// 3. An accessor property is never readable as `o.prop`, before or after its
//    setter runs; the getter and serializeJSON still see it.
acc = new ident452.Acc452();
assertThrows("accessor property is not readable before set", function() { return acc.color; });
acc.setColor("red");
assertThrows("accessor property is not readable after set", function() { return acc.color; });
assertThrows("accessor default is not readable", function() { return acc.size; });
assert("getter reads the value", acc.getColor(), "red");
assert("default via getter", acc.getSize(), "3");
assert("structKeyExists does not see it", structKeyExists(acc, "color"), false);
assert("isDefined does not see it", isDefined("acc.color"), false);
assert("serializeJSON includes accessor values",
	serializeJSON(acc), '{"color":"red","size":"3"}');

// 4. The top of every metadata chain is Lucee's implicit base component.
md = getMetadata(rel);
assert("standalone component extends the base", md.extends.name, "org.lucee.cfml.Component");
assert("base has no further extends", structKeyExists(md.extends, "extends"), false);
assert("base functions are empty", arrayLen(md.extends.functions), 0);
assert("base properties are empty", arrayLen(md.extends.properties), 0);
cmd = getMetadata(child);
depth = 0;
top = cmd;
while (structKeyExists(top, "extends")) {
	top = top.extends;
	depth++;
	if (depth > 10) break;
}
assert("extends walk terminates at the base", depth, 2);
assert("extends walk ends on the base", top.name, "org.lucee.cfml.Component");
fnNames = [];
walk = cmd;
while (structKeyExists(walk, "extends")) {
	for (f in walk.functions) arrayAppend(fnNames, f.name);
	walk = walk.extends;
}
arraySort(fnNames, "textnocase");
assert("inherited-function collector over the chain",
	arrayToList(fnNames), "bye,hello,makeNested,makeSibling");
assert("getComponentMetadata has the base too",
	getComponentMetadata("ident452.Plain452").extends.name, "org.lucee.cfml.Component");
assert("getComponentMetadata resolves the base by name",
	getComponentMetadata("org.lucee.cfml.Component").name, "org.lucee.cfml.Component");

suiteEnd();
</cfscript>
