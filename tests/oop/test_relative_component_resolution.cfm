<cfscript>
suiteBegin("OOP: relative (bare) component name resolution");

// Background: A bare (unqualified) component name in
// createObject("component", "Sibling") -- or new Sibling() -- must resolve
// relative to the CALLING CFC's package first. Maker lives in oop.relcomp
// alongside Sibling, so from inside Maker the bare name "Sibling" should
// find oop.relcomp.Sibling. RustCFML 0.153.0 does not search the caller
// package and throws "Could not find the component [Sibling]" -- viaCreate()
// catches it and returns "ERR:...". Lucee/ACF/BoxLang return the sibling
// marker "SIBLING-OK".
//
// This is the deepest blocker for the Wheels migrator TableDefinition DSL:
// createTable / t.string / t.integer / t.references / t.timestamps / t.create
// and changeTable / addColumn all route through relative createObject of
// sibling migrator components, so every one of them fails on RustCFML.
//
// The `new Sibling()` spelling has the identical defect on RustCFML, but
// there the resolution failure is UNCATCHABLE (escapes try/catch and would
// abort the file), so it is intentionally NOT asserted here -- only the
// catchable, runner-safe createObject form is. The dotted control is
// resolved from THIS (test-docroot) scope, not from inside Maker, because
// a dotted path is itself resolved relative to the caller package.

relcompMaker = createObject("component", "oop.relcomp.Maker").init();

// Control: fully-qualified resolution works on RustCFML and Lucee alike.
// Resolved from the test docroot scope (not from inside Maker).
controlSibling = "?";
try {
    controlSibling = createObject("component", "oop.relcomp.Sibling").hi();
} catch (any e) {
    controlSibling = "ERR:" & e.message;
}
assert(
    "qualified createObject('component','oop.relcomp.Sibling') resolves (control)",
    controlSibling,
    "SIBLING-OK"
);

// Gap: bare name via createObject, called from inside Maker, must resolve
// against Maker's own package (oop.relcomp).
assert(
    "bare createObject('component','Sibling') resolves relative to caller package",
    relcompMaker.viaCreate(),
    "SIBLING-OK"
);

suiteEnd();
</cfscript>
