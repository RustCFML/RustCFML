<cfscript>
suiteBegin("XmpParse");

// ============================================================
// XmpParse(xmpXml) — pure-Rust XMP (RDF/XML) metadata flattener.
//
// Replaces Adobe's xmpcore.jar (com.adobe.xmp.XMPMetaFactory) which Preside's
// XmpMetaReader.cfc used only to parse-and-enumerate. Output mirrors XMPCore's
// iterator: one entry per leaf property, keyed by its canonical path
// (prefix:localName, prefix:localName[N] for array items, .../child for structs).
//
// Namespace-URI `#` characters are doubled (`##`) only because these packets are
// CFML *string literals* here (`#` is the interpolation delimiter); real callers
// pass a runtime string with no such escaping.
// ============================================================

// ---- simple properties: attribute-form AND element-form ----
xmp1 = '<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="rdf">
  <rdf:Description rdf:about="" xmlns:dc="dc" xmlns:tiff="tiff"
     tiff:Make="Canon" tiff:Model="EOS 5D">
    <dc:format>image/jpeg</dc:format>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>';
m1 = xmpParse(xmp1);
assert("attr-form tiff:Make", m1["tiff:Make"], "Canon");
assert("attr-form tiff:Model", m1["tiff:Model"], "EOS 5D");
assert("element-form dc:format", m1["dc:format"], "image/jpeg");
assertFalse("rdf:about is not a property", structKeyExists(m1, "rdf:about"));

// ---- arrays: rdf:Seq / rdf:Bag / rdf:Alt ----
xmp2 = '<rdf:Description xmlns:rdf="rdf" xmlns:dc="dc">
  <dc:creator><rdf:Seq><rdf:li>Jane Doe</rdf:li><rdf:li>John Roe</rdf:li></rdf:Seq></dc:creator>
  <dc:subject><rdf:Bag><rdf:li>sky</rdf:li><rdf:li>sea</rdf:li></rdf:Bag></dc:subject>
  <dc:title><rdf:Alt><rdf:li xml:lang="x-default">My Photo</rdf:li></rdf:Alt></dc:title>
</rdf:Description>';
m2 = xmpParse(xmp2);
assert("Seq item 1 path", m2["dc:creator[1]"], "Jane Doe");
assert("Seq item 2 path (1-based)", m2["dc:creator[2]"], "John Roe");
assert("Bag item 1", m2["dc:subject[1]"], "sky");
assert("Bag item 2", m2["dc:subject[2]"], "sea");
assert("Alt item (lang qualifier stripped)", m2["dc:title[1]"], "My Photo");

// ---- struct: rdf:parseType="Resource" ----
xmp3 = '<rdf:Description xmlns:rdf="rdf" xmlns:xmpMM="mm" xmlns:stRef="stRef">
  <xmpMM:DerivedFrom rdf:parseType="Resource">
    <stRef:instanceID>xmp.iid:abc</stRef:instanceID>
    <stRef:documentID>xmp.did:def</stRef:documentID>
  </xmpMM:DerivedFrom>
</rdf:Description>';
m3 = xmpParse(xmp3);
assert("struct field instanceID", m3["xmpMM:DerivedFrom/stRef:instanceID"], "xmp.iid:abc");
assert("struct field documentID", m3["xmpMM:DerivedFrom/stRef:documentID"], "xmp.did:def");

// ---- multiple rdf:Description blocks are all flattened ----
xmp4 = '<rdf:RDF xmlns:rdf="rdf">
  <rdf:Description xmlns:dc="dc" dc:format="image/png"/>
  <rdf:Description xmlns:xmp="xmp" xmp:Rating="5"/>
</rdf:RDF>';
m4 = xmpParse(xmp4);
assert("desc block 1", m4["dc:format"], "image/png");
assert("desc block 2", m4["xmp:Rating"], "5");

// ---- end-to-end: reproduce Preside's XmpMetaReader flatten transform ----
extracted = {};
for (path in m2) {
    value = m2[path];
    if (len(trim(path)) && len(trim(value))) {
        p = listRest(path, ":");
        p = reReplace(p, "\[[0-9]+\]", "", "all");
        extracted[p] = value;
    }
}
// Preside strips prefix + indices; array items collapse to the LAST (last-wins),
// exactly as the XMPCore-backed original did.
assert("Preside flatten: title", extracted["title"], "My Photo");
assert("Preside flatten: creator collapses to last", extracted["creator"], "John Roe");
assert("Preside flatten: subject collapses to last", extracted["subject"], "sea");

// ---- empty / non-XMP input is handled gracefully ----
assert("empty string -> empty struct", structCount(xmpParse("")), 0);
assert("no rdf:Description -> empty struct", structCount(xmpParse("<a><b>x</b></a>")), 0);

suiteEnd();
</cfscript>
