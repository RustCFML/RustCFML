<cfscript>
suiteBegin("OOP: inherited relative implements path");

// ============================================================
// Background
// ============================================================
// A component may INHERIT an `implements="<relative.path>"` clause from an
// ancestor that lives in a DIFFERENT directory than the concrete subclass.
// The relative interface path must resolve against the directory of the cfc
// that DECLARED the clause (the ancestor), not the leaf subclass's directory.
//
// RustCFML previously resolved the inherited interface path against the leaf's
// own source directory, so ColdBox's boot failed: AbstractCacheBoxProvider
// (system/cache/) declares implements="providers.ICacheProvider", and the
// concrete LuceeProvider (system/cache/providers/) inherits it — resolution
// looked for a spurious system/cache/providers/providers/ICacheProvider and
// threw "Interface 'providers.ICacheProvider' not found".
//
// Fixtures mirror that shape without a mapping (so it passes on both engines):
//   tests/oop/ConcreteIfaceProvider.cfc          extends the abstract (below)
//   tests/oop/ifaceinherit/AbstractIfaceProvider.cfc  implements="sub.IIfaceThing"
//   tests/oop/ifaceinherit/sub/IIfaceThing.cfc        the interface
// The leaf is a directory ABOVE the abstract, so a leaf-relative resolution of
// "sub.IIfaceThing" would (wrongly) look in tests/oop/sub/ and fail.
// ============================================================

o = "";
err = "";
try {
	o = new ConcreteIfaceProvider();
} catch (any e) {
	err = e.message;
}

assert("concrete subclass with an inherited relative implements instantiates", err, "");
assert("it is an object", isObject(o), true);
assert("the inherited interface method runs", o.doThing(), "done");
assert("isInstanceOf sees the inherited interface", isInstanceOf(o, "IIfaceThing"), true);

suiteEnd();
</cfscript>
