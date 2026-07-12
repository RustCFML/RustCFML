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

// When the component is loaded via a MAPPING whose name differs from the
// physical directory (`/dotdotprobe` -> the `oop/` dir), Lucee/ACF qualify the
// relative `new Widget229()` with the MAPPING name the caller was loaded under
// ("dotdotprobe.pkg229.Widget229"), NOT the physical dir ("oop.pkg229..."). The
// prior filesystem-based package derivation dropped the mapping prefix, so
// getMetadata().name and isInstanceOf(FQN) diverged from Lucee (this broke
// Preside's AdapterFactory, loaded as preside.system...AdapterFactory, doing
// `new MySqlAdapter()`). Verified vs Lucee 7.0.4.
makerMapped = new dotdotprobe.pkg229.Maker();
wMapped = makerMapped.make();
assert("mapping-loaded caller: metadata.name keeps the mapping prefix",
    getMetadata(wMapped).name, "dotdotprobe.pkg229.Widget229");
assert("mapping-loaded caller: isInstanceOf matches mapping-qualified FQN",
    isInstanceOf(wMapped, "dotdotprobe.pkg229.Widget229"), true);

suiteEnd();
</cfscript>
