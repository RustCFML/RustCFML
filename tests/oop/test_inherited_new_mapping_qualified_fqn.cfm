<cfscript>
suiteBegin("OOP: unqualified new X() in inherited method keeps the mapping prefix");

// ChildMap (loaded via /dotdotprobe -> oop/) extends BaseMap, whose logical
// package is dotdotprobe.mapfqn.sys (mapping name != physical dir "oop").
// BaseMap.makeSibling() does an unqualified `new SiblingMap()`. Lucee names the
// new instance after the DEFINING file's mapping-qualified package, so
// metadata.name is dotdotprobe.mapfqn.sys.SiblingMap — NOT the webroot-relative
// oop.mapfqn.sys.SiblingMap the on-disk layout would produce. This is exactly
// TestBox's `new Expectation()` in testbox.system.BaseSpec asserting
// toBeInstanceOf("testbox.system.Expectation"); the webroot derivation used to
// drop the "testbox" mapping prefix.
child   = new dotdotprobe.mapfqn.specs.ChildMap();
sibling = child.makeSibling();

assert("metadata.name keeps the mapping prefix of the DEFINING class",
    getMetadata( sibling ).name, "dotdotprobe.mapfqn.sys.SiblingMap");

assert("siblingName() (getMetadata inside the defining file) agrees",
    child.siblingName(), "dotdotprobe.mapfqn.sys.SiblingMap");

// isInstanceOf is path-aware: the fully-qualified mapping name matches...
assertTrue("isInstanceOf matches the mapping-qualified FQN",
    isInstanceOf( sibling, "dotdotprobe.mapfqn.sys.SiblingMap" ));

// ...the wrong package does NOT match (Lucee parity — no loose suffix match).
assertFalse("isInstanceOf rejects the webroot-relative (wrong) package",
    isInstanceOf( sibling, "oop.mapfqn.sys.SiblingMap" ));

suiteEnd();
</cfscript>
