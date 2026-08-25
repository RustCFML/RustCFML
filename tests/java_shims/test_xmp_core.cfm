<cfscript>
suiteBegin("java shim: com.adobe.xmp.XMPMetaFactory (Adobe XMPCore)");

// Preside's XmpMetaReader.cfc reads image XMP with exactly this call shape. The
// jar path is passed and ignored — parsing is the native xmpParse() builtin.
xmp = '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns##">'
    & '<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/">'
    & '<dc:title><rdf:Alt><rdf:li xml:lang="x-default">A Photo</rdf:li></rdf:Alt></dc:title>'
    & '<dc:creator><rdf:Seq><rdf:li>First Author</rdf:li><rdf:li>Second Author</rdf:li></rdf:Seq></dc:creator>'
    & '<photoshop:Credit>Pixl8</photoshop:Credit>'
    & '</rdf:Description></rdf:RDF></x:xmpmeta>';

factory = CreateObject( "java", "com.adobe.xmp.XMPMetaFactory", [ "/no/such/xmpcore.jar" ] );
meta    = factory.parseFromString( trim( xmp ) );

// The reader loop, verbatim from XmpMetaReader.readMeta().
iterator  = meta.iterator();
extracted = {};
paths     = [];
while( iterator.hasNext() ) {
	prop  = iterator.next();
	path  = prop.getPath();
	value = prop.getValue();
	arrayAppend( paths, path );
	if ( len( trim( path ?: "" ) ) && len( trim( value ?: "" ) ) ) {
		path = listRest( path, ":" );
		path = reReplace( path, "\[[0-9]+\]", "", "all" );
		extracted[ path ] = value;
	}
}

assert( "title extracted", extracted.title ?: "", "A Photo" );
assert( "simple property extracted", extracted.Credit ?: "", "Pixl8" );
// Both rdf:Seq items are yielded as separate indexed paths; Preside's own path
// rewrite collapses them, last one winning. Matches XMPCore.
assert( "array items are yielded individually", arrayLen( paths ), 4 );
assert( "array item paths carry a 1-based index", paths[ 2 ], "dc:creator[1]" );
assert( "last array item wins after Preside's index strip", extracted.creator ?: "", "Second Author" );

// getNamespace() returns the schema URI, not the prefix.
it2 = meta.iterator();
assert( "getNamespace returns the schema URI", it2.next().getNamespace(), "http://purl.org/dc/elements/1.1/" );

// A fresh iterator restarts; the shared one is exhausted.
assertFalse( "an exhausted iterator reports no more", iterator.hasNext() );
assertThrows( "next() past the end throws NoSuchElementException", function(){ iterator.next(); } );

// Convenience getters.
assert( "getPropertyString by bare name", meta.getPropertyString( "http://purl.org/dc/elements/1.1/", "title" ), "A Photo" );
assertTrue( "doesPropertyExist true for a present property", meta.doesPropertyExist( "", "Credit" ) );
assertFalse( "doesPropertyExist false for an absent one", meta.doesPropertyExist( "", "NoSuchProperty" ) );

// Malformed input raises the exception type callers catch.
badType = "";
try {
	CreateObject( "java", "com.adobe.xmp.XMPMetaFactory" ).parseFromString( "not xml at all <<" );
} catch ( any e ) {
	badType = e.type;
}
assert( "a malformed packet raises XMPException", badType, "com.adobe.xmp.XMPException" );

// Anything past parse-and-iterate refuses rather than answering wrongly.
unsupportedType = "";
try {
	meta.serializeToString();
} catch ( any e ) {
	unsupportedType = e.type;
}
assert( "unsupported XMPCore methods throw", unsupportedType, "java.lang.UnsupportedOperationException" );

suiteEnd();
</cfscript>
