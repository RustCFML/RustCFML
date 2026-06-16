<cfscript>
suiteBegin("Encoding: EncodeForHTMLAttribute must encode attribute-dangerous chars (space and =)");

// Background: in HTML-attribute context the OWASP encoders (Lucee/Adobe ESAPI) encode every
// non-alphanumeric char below U+0100 — including space (-> &##x20;) and = (-> &##x3d;) — because an
// UNQUOTED attribute value can be broken out of with a raw space + = (attribute / event-handler
// injection). RustCFML 0.190.0 leaves space and = RAW in EncodeForHTMLAttribute output. The critical
// set < > & " ' IS encoded (so QUOTED usage is safe); the exposure is unquoted-attribute breakout.

// --- CONTROL (green on both engines): angle brackets are encoded (no raw < or >) ---
assertTrue("CONTROL: < and > are encoded (no raw angle brackets)",
    reFind("[<>]", EncodeForHTMLAttribute("a<b>c")) == 0);

// --- the gap: space must be encoded in attribute context (no raw space remains) ---
assertTrue("EncodeForHTMLAttribute encodes space (no raw space in output)",
    findNoCase(" ", EncodeForHTMLAttribute("a b")) == 0);

// --- the gap: = must be encoded in attribute context (no raw = remains) ---
assertTrue("EncodeForHTMLAttribute encodes = (no raw = in output)",
    findNoCase("=", EncodeForHTMLAttribute("a=b")) == 0);

suiteEnd();
</cfscript>
