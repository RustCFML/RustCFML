<cfscript>
suiteBegin("OOP: bare component in an inherited method resolves to the defining package even when invoked via a child-defined method");

// Background: a bare CreateObject("component","X") inside a method must resolve
// X relative to the package of the component that LEXICALLY DEFINES the method.
// That must hold no matter how the method is reached — including when a
// child-defined method on a subclass (in a different directory) calls the
// inherited method. Lucee/ACF/BoxLang do this. RustCFML 0.161.0 resolves the
// bare name relative to the OUTERMOST child frame's directory in that case.
//
// IndBase (oop/indbase/) defines makeSibling() -> CreateObject("component",
// "IndSibling") [sibling in oop/indbase/]. IndChild (oop/indleaf/) extends it
// and adds go() -> makeSibling().
//
//   childInstance.makeSibling()  (inherited method called DIRECTLY)  -> resolves on BOTH (the #133 fix; CONTROL)
//   childInstance.go()           (child method calls the inherited)  -> RustCFML 0.161: "Could not find the component [IndSibling]"; Lucee: resolves
//
// PR #133 fixed the direct-call case; this is the remaining indirect case.
// Why it matters: the Wheels migrator runs EXACTLY this shape. A user migration
// in app/migrator/migrations/ extends wheels.migrator.Migration; the migration's
// own up() calls the inherited createTable(), which does
// CreateObject("component","TableDefinition") (sibling in vendor/wheels/migrator/).
// On RustCFML createTable() resolves TableDefinition against the migration's dir
// and throws, so the migrator cannot create a table (Migration.cfc lines 62/76/
// 89/353; TableDefinition.cfc ColumnDefinition/ForeignKeyDefinition).

ibcvChild = createObject("component", "oop.indleaf.IndChild");

// --- CONTROL (green on both engines — the #133 fix): inherited method called DIRECTLY ---
assert("CONTROL: inherited makeSibling() called directly on the subclass resolves its sibling",
    ibcvChild.makeSibling(), "ind-sibling-ok");

// --- the gap: a child-defined method calls the inherited method ---
assert("child-defined go() calling the inherited makeSibling() resolves the bare sibling against the BASE package",
    ibcvChild.go(), "ind-sibling-ok");

suiteEnd();
</cfscript>
