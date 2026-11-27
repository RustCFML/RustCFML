<cfscript>
suiteBegin("XmlSearch XPath 1.0 (GH ##548)");

doc = xmlParse( '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>one</w:t></w:r></w:p><w:p><w:r><w:t>two</w:t></w:r></w:p></w:body></w:document>' );

// --- issue #548 repro ---
assert( "//w:t", arrayLen( xmlSearch( doc, "//w:t" ) ), 2 );
assert( "//*[local-name()='t']", arrayLen( xmlSearch( doc, "//*[local-name()='t']" ) ), 2 );
assert( "//*[name()='w:t']", arrayLen( xmlSearch( doc, "//*[name()='w:t']" ) ), 2 );

// --- positional path ---
second = xmlSearch( doc, "//w:p[2]/w:r/w:t" );
assert( "//w:p[2]/w:r/w:t count", arrayLen( second ), 1 );
assert( "//w:p[2]/w:r/w:t text", second[ 1 ].xmlText, "two" );

// --- attributes ---
attrDoc = xmlParse( '<root><item id="a"/><item id="b"/></root>' );
assert( "//@id count", arrayLen( xmlSearch( attrDoc, "//@id" ) ), 2 );
assert( "//@id first name", xmlSearch( attrDoc, "//@id" )[ 1 ].xmlName, "id" );
assert( "//@id first value", xmlSearch( attrDoc, "//@id" )[ 1 ].xmlText, "a" );

// --- non-node result: count() ---
assert( "count(//w:t)", xmlSearch( doc, "count(//w:t)" ), 2 );
assert( "count(//item)", xmlSearch( attrDoc, "count(//item)" ), 2 );

// --- union ---
assert( "union //w:t | //w:p", arrayLen( xmlSearch( doc, "//w:t | //w:p" ) ), 4 );

// --- boolean / predicate combinations ---
assert(
	"contains/starts-with/text predicates",
	arrayLen( xmlSearch( doc, "//w:t[contains(text(),'o') and starts-with(.,'t') or .='one']" ) ),
	2
);
assert(
	"not(contains)",
	arrayLen( xmlSearch( doc, "//w:t[not(contains(.,'z'))]" ) ),
	2
);
assert(
	"namespace-uri on prefixed element",
	arrayLen( xmlSearch( doc, "//*[namespace-uri()='http://schemas.openxmlformats.org/wordprocessingml/2006/main' and local-name()='t']" ) ),
	2
);

// --- parent axis ---
parents = xmlSearch( doc, "//w:t/.." );
assert( "parent of w:t", arrayLen( parents ), 2 );
assert( "parent is w:r", parents[ 1 ].xmlName, "w:r" );

// --- self / simple absolute path ---
plain = xmlParse( "<root><item>x</item><item>y</item></root>" );
assert( "/root/item", arrayLen( xmlSearch( plain, "/root/item" ) ), 2 );
assert( ".", arrayLen( xmlSearch( plain.xmlRoot, "." ) ), 1 );
assert( ". xmlName", xmlSearch( plain.xmlRoot, "." )[ 1 ].xmlName, "root" );

// --- string / boolean / number expressions ---
assert( "string(//w:t)", xmlSearch( doc, "string(//w:t)" ), "one" );
assertTrue( "boolean(//w:t)", xmlSearch( doc, "boolean(//w:t)" ) );
assertFalse( "boolean(//missing)", xmlSearch( doc, "boolean(//nope)" ) );
assert( "string-length(string(//w:t))", xmlSearch( doc, "string-length(string(//w:t))" ), 3 );

// --- invalid XPath must throw, not return [] ---
assertThrows( "unterminated function throws", function() {
	xmlSearch( doc, "//*[local-name(" );
} );
assertThrows( "unsupported axis throws", function() {
	xmlSearch( doc, "//foo[following::bar]" );
} );

suiteEnd();
</cfscript>
