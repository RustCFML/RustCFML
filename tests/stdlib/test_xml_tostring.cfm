<cfscript>
// GH #277 — toString()/string-coercion of an XML document must serialize to XML
// markup (Lucee parity), not throw "Can't cast Complex Object Type [Struct]".
// Exact forms verified against Lucee 7.0.4.
suiteBegin("XML toString / string-coercion (GH ##277)");

x = xmlParse("<root><item attr=""value""/><child>hello</child></root>");

// document node → declaration (with standalone) + serialized root
assert(
      "toString(xml document) serializes to markup"
    , toString(x)
    , '<?xml version="1.0" encoding="UTF-8" standalone="no"?><root><item attr="value"/><child>hello</child></root>'
);

// & concat and #interpolation# go through the same coercion path
assert("string concat coerces XML to markup", "" & x, toString(x));
assert("interpolation coerces XML to markup", "#x#", toString(x));

// deterministic: two identical parses stringify identically (TestBox isEqual relies on this)
y = xmlParse("<root><item attr=""value""/><child>hello</child></root>");
assertTrue("identical XML docs stringify equal", toString(x) eq toString(y));

// entity-escaping of text and attributes
z = xmlParse("<a b=""1"">text &amp; more</a>");
assert(
      "text & attributes are entity-escaped"
    , toString(z)
    , '<?xml version="1.0" encoding="UTF-8" standalone="no"?><a b="1">text &amp; more</a>'
);

// element node (not the document) → declaration without standalone
assert(
      "toString(xml element) serializes without standalone"
    , toString(x.xmlRoot)
    , '<?xml version="1.0" encoding="UTF-8"?><root><item attr="value"/><child>hello</child></root>'
);

suiteEnd();
</cfscript>
