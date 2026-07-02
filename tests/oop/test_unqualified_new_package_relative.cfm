<cfscript>
suiteBegin("OOP: unqualified new X() qualified by defining component's package (GH ##229)");

// Maker (package oop.pkg229) does `new Widget229()` unqualified. On Lucee/ACF
// that resolves relative to Maker's OWN package, so the resulting instance's
// metadata.name is the fully-qualified "oop.pkg229.Widget229" and isInstanceOf
// against that FQN is true. Previously rustcfml stamped the bare "Widget229",
// so the FQN check failed (this broke TestBox's Expectation FQN check).

maker = new oop.pkg229.Maker();
w = maker.make();

assert("metadata.name is package-qualified",
    getMetadata(w).name, "oop.pkg229.Widget229");
assert("isInstanceOf matches fully-qualified name",
    isInstanceOf(w, "oop.pkg229.Widget229"), true);
assert("isInstanceOf still matches bare name",
    isInstanceOf(w, "Widget229"), true);

suiteEnd();
</cfscript>
