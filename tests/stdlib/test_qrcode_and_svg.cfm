<cfscript>
suiteBegin("qrCodeGenerate() and imageReadSvg()");

// Two capabilities CFML had no primitive for, so applications reached for jars:
// QR encoding (net.glxn.qrgen) and SVG rasterisation (Apache Batik). Both are
// now builtins, with the java shims as thin adapters over them.

// ---- qrCodeGenerate -------------------------------------------------------
// NOTE ON WHAT THIS CAN AND CANNOT ASSERT. CFML has no QR *decoder*, so these
// tests check structure, determinism and variation. That the output actually
// DECODES to the right text was verified out of band with an independent
// decoder (the `rqrr` crate) against four codes including the real TOTP
// enrolment URL below — a decoder was deliberately NOT added as a dependency,
// because rqrr ships three licence files and that shape has made the
// THIRD-PARTY.txt CI gate flaky before. If you change the encoder, re-run that
// check by hand; these assertions will not catch a wrong payload.

otpUrl = "otpauth://totp/Preside:sysadmin?secret=GEZDGNBVGY3TQOJQ&issuer=Preside";

png = qrCodeGenerate( otpUrl, 125, "png" );
gif = qrCodeGenerate( otpUrl, 125, "gif" );
assertTrue( "a QR code comes back as binary", isBinary( png ) );
assert( "png carries the PNG magic", ucase( left( binaryEncode( png, "hex" ), 8 ) ), "89504E47" );
assert( "gif carries the GIF magic", ucase( left( binaryEncode( gif, "hex" ), 8 ) ), "47494638" );

assert( "the image is exactly the requested size", imageInfo( imageNew( png ) ).width, 125 );
assert( "and square", imageInfo( imageNew( png ) ).height, 125 );
assert( "a different size is honoured", imageInfo( imageNew( qrCodeGenerate( otpUrl, 300 ) ) ).width, 300 );

// Determinism, and variation. Between them these catch the two ways a QR
// encoder fails silently: emitting the same code for everything, and emitting
// something different every time.
assert( "the same input encodes identically"
      , hash( binaryEncode( png, "hex" ) ), hash( binaryEncode( qrCodeGenerate( otpUrl, 125, "png" ), "hex" ) ) );
assertTrue( "different text encodes differently"
          , hash( binaryEncode( png, "hex" ) )
            != hash( binaryEncode( qrCodeGenerate( otpUrl & "x", 125, "png" ), "hex" ) ) );
// A higher error-correction level needs a larger symbol for the same payload,
// so it must actually change the output — proof the argument is not ignored.
assertTrue( "the error-correction level changes the symbol"
          , hash( binaryEncode( qrCodeGenerate( otpUrl, 125, "png", "L" ), "hex" ) )
            != hash( binaryEncode( qrCodeGenerate( otpUrl, 125, "png", "H" ), "hex" ) ) );

assertThrows( "empty text is refused", function(){ qrCodeGenerate( "" ); } );
assertThrows( "an unknown format is refused", function(){ qrCodeGenerate( "x", 100, "xcf" ); } );

// ---- imageReadSvg ---------------------------------------------------------
svg = '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40" viewBox="0 0 80 40">'
    & '<rect width="80" height="40" fill="##0000ff"/>'
    & '<circle cx="20" cy="20" r="15" fill="##ff0000"/></svg>';

img = imageReadSvg( svg );
assert( "with no size, the SVG's own dimensions are used", imageInfo( img ).width, 80 );
assert( "…including the height", imageInfo( img ).height, 40 );

// One dimension given: the other follows the aspect ratio. An SVG is scalable,
// so inventing the missing edge and distorting it would be a strange default.
one = imageReadSvg( svg, 160 );
assert( "width alone scales proportionally", imageInfo( one ).width, 160 );
assert( "…and derives the height", imageInfo( one ).height, 80 );
tall = imageReadSvg( svg, 0, 200 );
assert( "height alone does the same", imageInfo( tall ).width, 400 );

// Both given: fit INSIDE the box, keeping aspect ratio and centring.
box = imageReadSvg( svg, 200, 200 );
assert( "both dimensions produce exactly that canvas", imageInfo( box ).width, 200 );
assert( "…on both edges", imageInfo( box ).height, 200 );

// The result is an ordinary image object, so the whole image family applies —
// that is the point of returning one rather than a one-shot converter.
resized = imageNew( "", 10, 10 );
imageResize( img, "40", "20" );
assert( "the result is a normal image object", imageInfo( img ).width, 40 );

// From a file, too.
p = getTempDirectory() & "/rustcfml_svg_" & createUUID() & ".svg";
fileWrite( p, svg );
fromFile = imageReadSvg( p, 320 );
assert( "an .svg path is read from disk", imageInfo( fromFile ).width, 320 );
out = getTempDirectory() & "/rustcfml_svg_" & createUUID() & ".png";
imageWrite( fromFile, out );
assert( "and writes out as a raster image", imageInfo( imageRead( out ) ).width, 320 );
fileDelete( p );
fileDelete( out );

assertThrows( "malformed SVG is refused", function(){ imageReadSvg( "<svg" ); } );
assertThrows( "empty input is refused", function(){ imageReadSvg( "" ); } );
assertThrows( "a missing file is refused", function(){ imageReadSvg( "/no/such/file.svg" ); } );

suiteEnd();
</cfscript>
