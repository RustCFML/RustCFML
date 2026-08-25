<cfscript>
suiteBegin("java shim: org.jsoup over the mutable HTML DOM");

// Preside uses jsoup in two places, both mutate-then-serialise:
//   EmailLoggingService.insertClickTrackingLinks — rewrite every <a href>
//   EmailStyleInliner                            — read <style>, write style attrs
// The adapter maps jsoup onto the HtmlDocument() builtin. An Element is the
// shared document handle plus an integer node handle, so a mutation through one
// element is visible through every other and in the document's output.

jsoup = CreateObject( "java", "org.jsoup.Jsoup" );

// ---- the click-tracking path ----------------------------------------------
doc = jsoup.parse( '<html><body><p>Hi</p>'
	& '<a href="https://example.com/a" title="First">Click A</a>'
	& '<a href="/relative">skip</a>'
	& '<a href="https://example.com/b">Click B</a></body></html>' );

links = doc.select( "A" );
assert( "select returns every matching element", arrayLen( links ), 3 );

// Elements is a plain array, so the CFML idioms a java.util.List supports on
// Lucee all work: index it, count it, iterate it.
assert( "an element is indexable and usable", links[ 1 ].tagName(), "a" );
counted = 0;
for ( l in links ) { counted++; }
assert( "and iterable", counted, 3 );

attribs = links[ 1 ].attributes();
assert( "Attributes.get reads an attribute", trim( attribs.get( "href" ) ), "https://example.com/a" );
assert( "and another", trim( attribs.get( "title" ) ), "First" );
// jsoup answers "" for an absent attribute, NOT null — callers Trim() the result.
assert( "an absent attribute is empty, not null", trim( links[ 2 ].attributes().get( "title" ) ), "" );
assert( "text() is the element's text", trim( links[ 1 ].text() ), "Click A" );

rewritten = 0;
for ( link in links ) {
	href = trim( link.attributes().get( "href" ) );
	if ( len( href ) && reFindNoCase( "^https?://", href ) ) {
		link.attr( "href", "https://track/?u=" & toBase64( href ) );
		rewritten++;
	}
}
assert( "only the absolute links were rewritten", rewritten, 2 );

out = doc.html();
assertTrue( "the mutation is visible in the document output", findNoCase( "https://track/?u=", out ) > 0 );
assertTrue( "the relative link was left alone", findNoCase( 'href="/relative"', out ) > 0 );
assertFalse( "and the original absolute href is gone", findNoCase( 'href="https://example.com/a"', out ) > 0 );

// outputSettings() is a fluent presentation knob this serialiser does not have;
// it is accepted and ignored so the caller's chain still runs (known-issues §66).
doc.outputSettings().charset( "ASCII" );
assertTrue( "outputSettings().charset() is inert but chainable", len( doc.html() ) > 0 );

// ---- the style-inlining path ----------------------------------------------
styleHtml = '<html><head><style>a { color: blue; } p { margin: 0; }</style></head>'
	& '<body><p style="font-size:12px">x</p></body></html>';
doc2 = jsoup.parse( styleHtml );

styleElements = doc2.select( "style" );
assert( "the style block is found", arrayLen( styleElements ), 1 );

// getAllElements().get( 0 ) is jsoup's "this element" — the first entry is self.
selfEl = styleElements[ 1 ].getAllElements().get( 0 );
assert( "getAllElements()[0] is the element itself", selfEl.tagName(), "style" );

// data() is the RAW content of a <style>: not whitespace-collapsed and not
// escaped. Collapsing it would change what the stylesheet means.
assert( "data() returns the CSS byte for byte"
      , selfEl.data(), "a { color: blue; } p { margin: 0; }" );

// hashCode is an identity: stable for one element, distinct between two.
// _getElementsWithStylesToApply uses it as a struct key to group per element.
paras = doc2.select( "p" );
assert( "hashCode is stable across two reads", paras[ 1 ].hashCode(), paras[ 1 ].hashCode() );
assertTrue( "and differs between two elements"
          , paras[ 1 ].hashCode() != styleElements[ 1 ].hashCode() );

assert( "an existing inline style is readable", paras[ 1 ].attr( "style" ), "font-size:12px" );
paras[ 1 ].attr( "style", "font-size:12px;color:red;" );
assertTrue( "writing the style attribute reaches the document"
          , findNoCase( "color:red", doc2.toString() ) > 0 );

// Element.html() is the element's INNER html; Document.html() is the whole
// document. jsoup draws exactly that distinction and the inliner relies on it.
bodyEl = doc2.select( "body" )[ 1 ];
assert( "Element.html() is inner HTML"
      , bodyEl.html(), '<p style="font-size:12px;color:red;">x</p>' );
assertTrue( "Document.html() is the whole document", findNoCase( "<head>", doc2.html() ) > 0 );

// ---- the cache key --------------------------------------------------------
// readStyles keys its cache on Hash( styleElements.toString() ). If that did not
// vary with the CSS, one email's styles would be served for another — a silent
// wrong answer, so it is asserted directly.
blue  = jsoup.parse( '<html><head><style>a { color: blue; }</style></head><body>x</body></html>' );
green = jsoup.parse( '<html><head><style>a { color: green; }</style></head><body>x</body></html>' );
assertTrue( "an Elements array stringifies to its content"
          , findNoCase( "color: blue", blue.select( "style" ).toString() ) > 0 );
assertTrue( "so two different stylesheets produce different cache keys"
          , hash( blue.select( "style" ).toString() ) != hash( green.select( "style" ).toString() ) );

// ---- fragments ------------------------------------------------------------
// A widget is often a bare <td> or <tr>. Parsing it as a document would wrap it
// in html/head/body and hand back more than it was given.
frag = jsoup.parseBodyFragment( '<td class="cell">only me</td>' );
assertFalse( "a fragment does not gain html/body scaffolding"
           , findNoCase( "<body", frag.toString() ) > 0 );

// ---- refusals -------------------------------------------------------------
// Jsoup.clean() is a sanitiser with a Whitelist policy model that has no
// equivalent here; substituting AntiSamy would silently apply different rules.
cleanErr = "";
try { jsoup.clean( "<b>x</b>", "none" ); } catch ( any e ) { cleanErr = e.message; }
assertTrue( "Jsoup.clean() is refused and points at sanitizeHtml()"
          , findNoCase( "sanitizeHtml", cleanErr ) > 0 );
assertThrows( "an unmodelled jsoup method throws", function(){ doc.connect(); } );

suiteEnd();
</cfscript>
