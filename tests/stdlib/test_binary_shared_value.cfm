<cfscript>
suiteBegin("Binary values are shared until written (copy-on-write)");

// A Binary is now held in a shared Arc (like strings): reading or passing it no
// longer copies the bytes. An index write must still only affect the variable
// written through, as when every read made a copy.
a = charsetDecode( "ABC", "utf-8" );
assert( "a binary reads back", charsetEncode( a, "utf-8" ), "ABC" );

function takesBinary( required b ) { return arrayLen( arguments.b ); }
assert( "passing a binary to a function", takesBinary( a ), 3 );
assert( "objectLoad round trip through a variable", objectLoad( objectSave( a ) )[ 2 ], a[ 2 ] );

if ( isRustCFML() ) {
	// Lucee's binary is a Java byte[] (a reference: assignment aliases). RustCFML
	// has always given binaries value semantics; the shared storage must not
	// change that.
	b = a;
	b[ 1 ] = 90; // "Z"
	assert( "the written copy changed", charsetEncode( b, "utf-8" ), "ZBC" );
	assert( "the original is untouched", charsetEncode( a, "utf-8" ), "ABC" );

	function mutate( required x ) { arguments.x[ 3 ] = 89; return arguments.x; }
	c = mutate( a );
	assert( "the function's copy changed", charsetEncode( c, "utf-8" ), "ABY" );
	assert( "the caller's binary is untouched", charsetEncode( a, "utf-8" ), "ABC" );

	s = { bin = a };
	d = duplicate( s );
	d.bin[ 2 ] = 88;
	assert( "duplicate() then write leaves the source alone", charsetEncode( s.bin, "utf-8" ), "ABC" );
}

assertThrows( "writing past the end still throws", function() { var z = charsetDecode( "A", "utf-8" ); z[ 5 ] = 1; } );

suiteEnd();
</cfscript>
