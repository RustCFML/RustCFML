<cfscript>
suiteBegin("OOP: unqualified new X() in INHERITED method uses defining package (GH ##237)");

// BDDTest237 (package oop.pkg237.specs) inherits makeExpectation() from
// BaseSpec237 (package oop.pkg237.sys). That method does `new Expectation237()`
// unqualified. On Lucee/ACF the unqualified name resolves against the DEFINING
// component's package (oop.pkg237.sys), so metadata.name is
// oop.pkg237.sys.Expectation237 — NOT oop.pkg237.specs.Expectation237 (the
// runtime subclass's package, which is what a naive `this`-based qualification
// produced). This broke TestBox's BaseSpec building Expectation and asserting
// toBeInstanceOf("testbox.system.Expectation").
bdd = new oop.pkg237.specs.BDDTest237();
assert("unqualified new resolves to the defining file's package",
    bdd.run(), "oop.pkg237.sys.Expectation237");

// And the same-package case (issue 229) still qualifies correctly.
maker = new oop.pkg229.Maker();
w = maker.make();
assert("same-package new (##229 regression) still qualified",
    getMetadata(w).name, "oop.pkg229.Widget229");

suiteEnd();
</cfscript>
