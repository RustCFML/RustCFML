<cfscript>
suiteBegin("OOP: bare component name in an inherited method resolves to the defining component's package");

// Background: a bare (unqualified) CreateObject("component","Sibling") inside a
// method must resolve relative to the package of the component that LEXICALLY
// DEFINES the method, not the runtime instance's concrete-class package. When
// the method is inherited by a subclass that lives in a DIFFERENT directory,
// the bare name must still be found next to the PARENT. Lucee/ACF/BoxLang do
// this. RustCFML 0.160.0 resolves the bare name against the concrete subclass's
// directory instead, so it fails: "Could not find the component [InhSibling]".
//
// This is the inherited-method sibling of #132 (which fixed bare resolution
// when the caller IS the defining component, co-located with the sibling).
// Here InhParent (oop/inh/) defines viaCreate() -> CreateObject("component",
// "InhSibling"); InhChild (oop/inhsub/) is an empty subclass in another dir.
//
// Why it matters for Wheels: the migrator's TableDefinition DSL runs entirely
// through this shape. User migrations live in app/migrator/migrations/ and
// extend wheels.migrator.Migration; Migration.createTable() does a bare
// CreateObject("component","TableDefinition") to reach its package-sibling in
// vendor/wheels/migrator/. On RustCFML the inherited createTable() resolves
// "TableDefinition" against the user-migration directory and throws, so the
// migrator cannot create a single table (Migration.cfc TableDefinition x2 /
// ViewDefinition / ForeignKeyDefinition; TableDefinition.cfc ColumnDefinition /
// ForeignKeyDefinition). Surfaced running the migrator pristine on stock.

ibcrChild = createObject("component", "oop.inhsub.InhChild");
ibcrParent = createObject("component", "oop.inh.InhParent");

// --- the gap: inherited method on a subclass in another dir ---
assert("inherited viaCreate() on a subclass resolves the bare sibling against the PARENT package",
    ibcrChild.viaCreate(), "inh-sibling-ok");

// --- CONTROL (green on both engines): caller IS the defining component (the #132 case) ---
assert("CONTROL: viaCreate() called directly on the defining parent resolves the bare sibling",
    ibcrParent.viaCreate(), "inh-sibling-ok");

suiteEnd();
</cfscript>
