<cfscript>
suiteBegin("Tags: cfoutput encodeFor");

// <cfoutput encodeFor="..."> auto-encodes every #expr# in the body. The
// attribute was parsed and then dropped entirely — it appeared nowhere in the
// engine — so a page that asked for auto-encoding emitted raw markup and the
// XSS protection silently did not exist.
//
// These assertions deliberately pin the SECURITY PROPERTY (the dangerous
// characters are gone; literals and non-interpolated text are untouched) rather
// than exact encoder output. RustCFML's HTML/JavaScript codecs are OWASP/ESAPI-
// exact and encode MORE than Lucee 7's (see the note on
// fn_encode_for_html_attribute in builtins.rs — Adobe CF and BoxLang agree with
// us; Lucee is the outlier). Asserting Lucee's exact bytes here would either
// fail cross-engine or lock in the weaker encoding. `url` is the one codec that
// is byte-identical on both, so it is pinned exactly.

evil = '<b>x</b>&"q"';
num = 42;
</cfscript>

<cfsavecontent variable="plain"><cfoutput>#evil#</cfoutput></cfsavecontent>
<cfsavecontent variable="encHtml"><cfoutput encodeFor="html">#evil#</cfoutput></cfsavecontent>
<cfsavecontent variable="encJs"><cfoutput encodeFor="javascript">#evil#</cfoutput></cfsavecontent>
<cfsavecontent variable="encUrl"><cfoutput encodeFor="url">#evil#</cfoutput></cfsavecontent>
<cfsavecontent variable="litBody"><cfoutput encodeFor="html">literal <b>markup</b> #evil#</cfoutput></cfsavecontent>
<cfsavecontent variable="numBody"><cfoutput encodeFor="html">#num#</cfoutput></cfsavecontent>
<cfsavecontent variable="nested"><cfoutput encodeFor="html"><cfif true>#evil#</cfif></cfoutput></cfsavecontent>
<cfsavecontent variable="after"><cfoutput>#evil#</cfoutput></cfsavecontent>

<cfscript>
// Baseline: with no encodeFor the value goes out raw. If this ever stops being
// true the assertions below stop proving anything.
assert("without encodeFor the value is emitted raw", trim(plain), '<b>x</b>&"q"');

// The actual XSS control: no raw angle brackets or quotes survive.
assertTrue("encodeFor=html must not emit a raw <b> tag",
	!findNoCase("<b>", encHtml));
assertTrue("encodeFor=html must not emit a raw <",
	!find("<", encHtml));
assertTrue("encodeFor=html must not emit a raw >",
	!find(">", encHtml));
assertTrue("encodeFor=html must escape < as &lt;",
	findNoCase("&lt;", encHtml) GT 0);
assertTrue("encodeFor=html must change the output",
	trim(encHtml) NEQ trim(plain));

// The JS codecs differ in strictness — RustCFML escapes `<` as <, Lucee
// leaves it bare — so assert the property BOTH guarantee and that actually
// matters inside a <script> block: the sequence `</` cannot survive, so the
// value can never close the enclosing script element.
assertTrue("encodeFor=javascript must not emit a raw </ (script-breakout)",
	!find("</", encJs));
assertTrue("encodeFor=javascript must change the output",
	trim(encJs) NEQ trim(plain));

// URL is byte-identical across engines, so pin it exactly.
assert("encodeFor=url uses the URL codec",
	trim(encUrl), '%3Cb%3Ex%3C%2Fb%3E%26%22q%22');

// Only interpolations are encoded — literal markup in the body is untouched.
assertTrue("literal body markup must NOT be encoded",
	findNoCase("<b>markup</b>", litBody) GT 0);
assertTrue("the interpolated value inside a literal-bearing body must still be encoded",
	!findNoCase(">x</b>", litBody));

assert("a numeric value passes through unchanged", trim(numBody), "42");

// Nested tags inside the body inherit the encoding.
assert("interpolation inside a nested tag is encoded the same as at top level",
	trim(nested), trim(encHtml));

// The encoding must not leak past the closing tag.
assert("encodeFor does not leak into later output", trim(after), '<b>x</b>&"q"');

suiteEnd();
</cfscript>
