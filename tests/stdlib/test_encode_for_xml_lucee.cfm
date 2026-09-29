<cfscript>
suiteBegin("encodeForXML / encodeForXMLAttribute match Lucee (OWASP forXml)");
// These were byte-for-byte copies of xmlFormat. Lucee encodes quotes as
// numeric references, keeps tab/LF/CR, and replaces characters XML cannot
// carry with a space; the attribute form also leaves `>` alone. Expected
// values come from Lucee 7.1.

assert("quotes become numeric references", encodeForXML('"' & "'"), "&##34;&##39;");
assert("markup characters become named entities", encodeForXML("<a>&"), "&lt;a&gt;&amp;");
assert("tab, LF and CR are kept", encodeForXML(chr(9) & chr(10) & chr(13)), chr(9) & chr(10) & chr(13));
assert("C0 controls become a space", encodeForXML("a" & chr(1) & chr(31) & "b"), "a  b");
assert("DEL and C1 controls become a space", encodeForXML(chr(127) & chr(128) & chr(159)), "   ");
assert("noncharacters become a space", encodeForXML(chr(64976) & chr(65534) & chr(131071)), "   ");
assert("ordinary non-ASCII passes through", encodeForXML("é©" & chr(160)), "é©" & chr(160));
assert("xmlFormat keeps its named quote entities", xmlFormat('"' & "'"), "&quot;&apos;");

assert("attribute: > is left alone", encodeForXMLAttribute("a>b"), "a>b");
assert("attribute: < and & are encoded", encodeForXMLAttribute("<&"), "&lt;&amp;");
assert("attribute: quotes become numeric references", encodeForXMLAttribute('"' & "'"), "&##34;&##39;");
assert("attribute: whitespace is kept", encodeForXMLAttribute(chr(9) & chr(10)), chr(9) & chr(10));
assert("attribute: controls become a space", encodeForXMLAttribute(chr(1) & chr(127)), "  ");

suiteEnd();
</cfscript>
