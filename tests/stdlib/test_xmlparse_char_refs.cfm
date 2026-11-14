<cfscript>
// XmlParse decodes character references (`&#220;`, `&#xDC;`) in element text,
// and entity and character references in attribute values, as XML 1.0 and
// Lucee do (GH #519). Every expectation was read off Lucee 7.1.2.
suiteBegin("XmlParse: character and entity references");

x = xmlParse("<a>&##220;ber</a>");
assert("decimal reference", x.xmlRoot.xmlText, "Über");
assert("decoded to one character", asc(left(x.xmlRoot.xmlText, 1)), 220);
assert("hex reference", xmlParse("<a>&##xDC;ber</a>").xmlRoot.xmlText, "Über");
assert("astral character", xmlParse("<a>&##x1F600;!</a>").xmlRoot.xmlText, "😀!");
x3 = xmlParse("<si><t>&##220;berpr&##252;fung</t></si>");
assert("in a child element", x3.xmlRoot.xmlChildren[1].xmlText, "Überprüfung");
assert("named entities still decode", xmlParse("<a>&amp;ok</a>").xmlRoot.xmlText, "&ok");

y = xmlParse("<a t='x &amp; y &lt; z &##220;' u=""q&quot;r""/>");
assert("attribute: entities and a character reference", y.xmlRoot.xmlAttributes.t, "x & y < z Ü");
assert("attribute: &quot;", y.xmlRoot.xmlAttributes.u, 'q"r');

assertThrows("a reference to NUL is not well-formed", function() { xmlParse("<a>&##0;</a>"); });
assertThrows("a surrogate code point is not well-formed", function() { xmlParse("<a>&##xD800;</a>"); });

suiteEnd();
</cfscript>
