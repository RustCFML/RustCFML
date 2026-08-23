<cfscript>
suiteBegin("XML Functions");

// --- isXML ---
assertTrue("isXML valid document", isXML("<root><item>test</item></root>"));
assertFalse("isXML invalid string", isXML("not xml"));

// --- xmlParse ---
doc = xmlParse("<root><item>test</item></root>");
assertNotNull("xmlParse returns object", doc);

// --- xmlRoot access ---
rootNode = doc.xmlRoot;
assertNotNull("xmlRoot not null", rootNode);
assert("xmlRoot name", rootNode.xmlName, "root");

// --- child element access ---
children = rootNode.xmlChildren;
assertTrue("xmlChildren has elements", arrayLen(children) > 0);
assert("first child name", children[1].xmlName, "item");
assert("first child text", children[1].xmlText, "test");

// --- xmlSearch ---
results = xmlSearch(doc, "//item");
assertTrue("xmlSearch finds elements", arrayLen(results) > 0);

// --- named child access derefs to the FIRST element (GH #343) ---
// `node.Child.member` is THE standard CFML XML idiom. A named child is a GROUP
// (there can be several with one tag name), and reading a member off the group
// addresses the first of them — Lucee's XMLMultiElementStruct delegates every
// non-integer key to element 1. Returning nothing here was silent: a template
// reading `##doc.Config.Setting.xmlText##` rendered empty and looked like missing
// data rather than an engine fault.
gx = xmlParse("<Root><Kid A='1'>txt</Kid><Kid A='2'>two</Kid><Solo>s</Solo></Root>");
assert("named child derefs to first element", gx.Root.Kid.xmlText, "txt");
assert("named child attribute derefs", gx.Root.Kid.xmlAttributes.A, "1");
assert("named child name derefs", gx.Root.Kid.xmlName, "Kid");
// A single child behaves the same as a group of one.
assert("single named child derefs", gx.Root.Solo.xmlText, "s");
// Indexed access still addresses each sibling.
assert("indexed child 1", gx.Root.Kid[1].xmlText, "txt");
assert("indexed child 2", gx.Root.Kid[2].xmlText, "two");
// Deref chains through several levels of named children.
assert("nested named-child chain", xmlParse("<a><b><c>deep</c></b></a>").a.b.c.xmlText, "deep");
// Delegation is keyed on node SHAPE, so an ordinary array of structs is
// unaffected — a member read on one must not start resolving to element 1.
// RustCFML-only: Lucee reads a member on an array as an index and throws
// ("cannot cast [NOPE] string to a number value") before the guard can run, so
// asserting it there would abort the whole file rather than fail one line.
if ( isRustCFML() ) {
    assertTrue("plain array member read is still null", isNull( [ { q = 1 } ].nope ));
}

suiteEnd();
</cfscript>
