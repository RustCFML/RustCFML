<cfscript>
suiteBegin("OOP: getMetadata() must not propagate a parent's displayName onto a child's leaf metadata");

// Background: component attributes such as displayName are NOT inherited onto a
// subclass's own metadata struct. getMetadata(child) reports the child's own
// declared attributes; the parent's displayName lives on the PARENT's metadata
// (reachable via the metadata's `extends` chain), not copied onto the leaf.
// Lucee, Adobe CF and BoxLang all leave displayName ABSENT from a child's
// metadata when the child declares none of its own.
//
// RustCFML 0.161.0 copies the parent's displayName onto the child's leaf
// metadata, so getMetadata(child).displayName erroneously reports the parent's
// label as if the child declared it.
//
//   getMetadata(new MetaDisplayBase())  -> displayName="MetaDisplayBaseLabel" on BOTH (CONTROL)
//   getMetadata(new MetaDisplayChild()) -> Lucee: displayName ABSENT; RustCFML 0.161: present="MetaDisplayBaseLabel"
//
// Why it matters: framework introspection that walks getMetadata() to decide
// behavior per concrete component (Wheels integrates controller/model mixins
// and inspects component metadata at startup) must see the leaf's OWN
// attributes, not silently inherited ones — an inherited attribute leaking onto
// every subclass changes what introspection observes.

mdBase  = getMetadata(new oop.MetaDisplayBase());
mdChild = getMetadata(new oop.MetaDisplayChild());

// --- CONTROL (green on both engines): the parent DOES carry its own displayName ---
assertTrue("CONTROL: parent metadata carries its own displayName",
    structKeyExists(mdBase, "displayName") && mdBase.displayName == "MetaDisplayBaseLabel");

// --- CONTROL (green on both engines): the child's metadata still reports the inheritance link ---
assertTrue("CONTROL: child metadata exposes its extends chain to the parent",
    structKeyExists(mdChild, "extends"));

// --- the gap: the child declares no displayName, so its leaf metadata must not have one ---
assertFalse("child metadata must NOT carry the parent's inherited displayName",
    structKeyExists(mdChild, "displayName"));

suiteEnd();
</cfscript>
