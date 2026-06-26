<cfscript>
suiteBegin("getMetadata fidelity (ctor-time self name + ancestor functions/path)");

// ============================================================
// Gaps surfaced laddering the Wheels framework test suite on RustCFML 0.309.0.
// ============================================================
// (A) GetMetadata(this).name read DURING the pseudo-constructor returns the
//     literal "Anonymous" on RustCFML (it resolves correctly only AFTER
//     construction). Wheels keys its promote-includes memo on this ctor-time
//     name (Global.cfc), so the cache lands under "Anonymous" —
//     promoteIncludedGlobalsMemoSpec fails with "got [Anonymous]".
// (B) GetMetadata().extends ANCESTOR nodes must carry the same functions[] and
//     path keys the leaf component does (Lucee/ACF/BoxLang populate every level
//     of the chain). RustCFML leaves them off the ancestor node, so the Wheels
//     inheritance-chain config()-shadow scan (Controller.cfc
//     $configOverrideSkipsSuper) cannot see ancestor functions and
//     configSuperWarningSpec fails.
// ============================================================

// (A) ctor-time self name
ctorObj = createObject("component", "GmaCtorFixture");
assert("GetMetadata(this).name in the pseudo-constructor is the class name, not Anonymous", ctorObj.getNameAtCtor(), "GmaCtorFixture");

// (B) ancestor node carries functions[] and path
md = getMetadata(createObject("component", "GmaSubFixture"));
ext = md.keyExists("extends") ? md.extends : {};
assertTrue("GetMetadata().extends ancestor node carries a functions array", structKeyExists(ext, "functions"));
assertTrue("GetMetadata().extends ancestor node carries a path", structKeyExists(ext, "path"));

suiteEnd();
</cfscript>
