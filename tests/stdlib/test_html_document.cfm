<cfscript>
suiteBegin("HtmlDocument(): a mutable HTML document");

// CFML could already READ html (htmlParse() hands back an immutable, XML-shaped
// struct) but never change one and write it back. That is what rewriting links
// or inlining CSS needs, and why applications reached for jsoup through
// createObject("java", …). This is that capability under a CFML name.

doc = htmlDocument( '<div class="wrap" id="main">'
	& '<p style="color:red">Hello <b>world</b></p>'
	& '<a href="http://example.com/x" title="T">one</a>'
	& '<a href="https://example.com/y">two</a>'
	& '<style>a { color: blue; }  p { margin: 0; }</style></div>' );

// select() returns node HANDLES, not sub-objects: every operation goes back
// through the one document that owns the tree.
links = doc.select( "a" );
assert( "select finds both links", arrayLen( links ), 2 );
assert( "attr reads an attribute", doc.attr( links[ 1 ], "href" ), "http://example.com/x" );
// An absent attribute is "" rather than null, so string operations on the result
// do not blow up.
assert( "an absent attribute is empty", doc.attr( links[ 2 ], "title" ), "" );
assertTrue( "hasAttr sees a present one", doc.hasAttr( links[ 1 ], "title" ) );
assertFalse( "and not an absent one", doc.hasAttr( links[ 2 ], "title" ) );

// Mutation, and it must show up in the serialised document.
doc.setAttr( links[ 1 ], "href", "https://tracked/1" );
assertTrue( "setAttr reaches the document", findNoCase( "https://tracked/1", doc.toString() ) > 0 );
assertFalse( "and replaces the old value", findNoCase( "example.com/x", doc.toString() ) > 0 );

// Attribute names are case-insensitive in HTML, so writing "HREF" must land on
// the same attribute rather than adding a second one.
doc.setAttr( links[ 1 ], "HREF", "https://tracked/2" );
assert( "a differently-cased write updates in place", structCount( doc.attributes( links[ 1 ] ) ), 2 );
assert( "with the new value", doc.attr( links[ 1 ], "href" ), "https://tracked/2" );

doc.removeAttr( links[ 1 ], "title" );
assertFalse( "removeAttr drops it", doc.hasAttr( links[ 1 ], "title" ) );

// Source order is preserved through a parse/serialise round trip. Without it the
// parser sorts attributes alphabetically, which silently rewrites the caller's
// markup and destabilises anyone hashing the output.
divAttrs = doc.attributes( doc.select( "div" )[ 1 ] );
assert( "attributes come back in source order", structKeyList( divAttrs ), "class,id" );

// text() collapses whitespace the way a renderer would; data() does not, because
// collapsing a stylesheet changes what it means.
para = doc.select( "p" )[ 1 ];
assert( "text() is the collapsed text of the subtree", doc.text( para ), "Hello world" );
assert( "data() is byte-exact", doc.data( doc.select( "style" )[ 1 ] ), "a { color: blue; }  p { margin: 0; }" );

assert( "html() is the inner HTML", doc.html( para ), "Hello <b>world</b>" );
assert( "outerHtml() includes the element", doc.outerHtml( para ), '<p style="color:red">Hello <b>world</b></p>' );
assert( "tagName reports the tag", doc.tagName( para ), "p" );

// selectWithin scopes to descendants; allElements is self-plus-descendants.
assert( "selectWithin is scoped", arrayLen( doc.selectWithin( doc.select( "div" )[ 1 ], "a" ) ), 2 );
assert( "and finds nothing outside its root", arrayLen( doc.selectWithin( para, "a" ) ), 0 );
assert( "allElements starts with the element itself"
      , doc.tagName( doc.allElements( para )[ 1 ] ), "p" );

// ---- fragment vs document -------------------------------------------------
// A snippet must not come back wrapped in html/head/body scaffolding it never had.
frag = htmlDocument( '<p class="cell">only me</p>' );
assert( "a fragment round-trips unchanged", frag.toString(), '<p class="cell">only me</p>' );
assertFalse( "and gains no scaffolding", findNoCase( "<body", frag.toString() ) > 0 );

// An orphan table cell loses its tag — that is the HTML parsing algorithm, not a
// defect here: a <td> outside a table has no valid insertion point, and a browser
// does the same with `innerHTML = "<td>x</td>"`. Callers that hand round bare
// cells wrap them first (Preside's EmailStyleInliner does exactly this), and the
// wrapped form survives intact.
orphan = htmlDocument( '<td class="cell">only me</td>' );
assert( "an orphan <td> keeps its text but loses the tag", orphan.toString(), "only me" );
wrapped = htmlDocument( '<table><tbody><tr><td class="cell">only me</td></tr></tbody></table>' );
assertTrue( "wrapped in a table, the cell survives"
          , findNoCase( '<td class="cell">only me</td>', wrapped.toString() ) > 0 );

full = htmlDocument( '<html><head><title>T</title></head><body><p>x</p></body></html>' );
assertTrue( "a full document keeps its structure", findNoCase( "<head>", full.toString() ) > 0 );

// The mode can be forced when the sniff would guess wrong.
forced = htmlDocument( '<p>plain</p>', "document" );
assertTrue( "an explicit document parse adds the scaffolding", findNoCase( "<html>", forced.toString() ) > 0 );

// ---- errors ---------------------------------------------------------------
assertThrows( "an invalid CSS selector is reported", function(){ doc.select( "a[[[" ); } );
assertThrows( "a bogus node handle is reported", function(){ doc.attr( 99999, "href" ); } );
assertThrows( "an unknown method is reported", function(){ doc.notAMethod(); } );

suiteEnd();
</cfscript>
