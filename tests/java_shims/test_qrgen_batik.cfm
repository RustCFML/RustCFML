<cfscript>
suiteBegin("java shim: net.glxn.qrgen and org.apache.batik");

// Two jar-backed services Preside carries, now adapters over builtins:
//   QrCodeGenerator.cfc -> qrgen  -> qrCodeGenerate()
//   SvgToPngService.cfc -> batik  -> imageReadSvg() + imageWrite()

// ---- QRGen ----------------------------------------------------------------
// The fluent builder: nothing is computed until stream().
imageTypes = CreateObject( "java", "net.glxn.qrgen.core.image.ImageType", [ "/no/such/qrgen.jar" ] );
// ImageType is an enum whose members are read as FIELDS, not getters.
assert( "ImageType.GIF is readable as a field", imageTypes.GIF, "gif" );
assert( "ImageType.PNG too", imageTypes.PNG, "png" );

qrCode = CreateObject( "java", "net.glxn.qrgen.javase.QRCode", [ "/no/such/qrgen.jar" ] );
binary = qrCode.from( "https://example.com/enrol" )
               .to( imageTypes.GIF )
               .withSize( 125, 125 )
               .stream()
               .toByteArray();

// Preside's QrCodeGenerator declares `binary function`, so toByteArray() must
// yield a Binary here — unlike java.io.ByteArrayOutputStream's shim, whose
// signed-byte ARRAY form is what String.getBytes() and the TOTP path need.
assertTrue( "stream().toByteArray() is binary", isBinary( binary ) );
assert( "and is the requested format", ucase( left( binaryEncode( binary, "hex" ), 8 ) ), "47494638" );
assert( "at the requested size", imageInfo( imageNew( binary ) ).width, 125 );

// A QR symbol is square, so a non-square request cannot be honoured as asked;
// the smaller edge wins, keeping the code inside the box the caller reserved.
oblong = qrCode.from( "x" ).to( imageTypes.PNG ).withSize( 200, 80 ).stream().toByteArray();
assert( "a non-square request uses the smaller edge", imageInfo( imageNew( oblong ) ).width, 80 );

// The builder must not compute early, and must not leak between chains.
b1 = qrCode.from( "one" ).to( imageTypes.PNG ).withSize( 100, 100 ).stream().toByteArray();
b2 = qrCode.from( "two" ).to( imageTypes.PNG ).withSize( 100, 100 ).stream().toByteArray();
assertTrue( "two chains from the same class object are independent"
          , hash( binaryEncode( b1, "hex" ) ) != hash( binaryEncode( b2, "hex" ) ) );

assertThrows( "stream() without from() is refused", function(){ qrCode.stream(); } );
// file() writes to a JVM temp File; the adapter points at the alternative
// rather than inventing a path.
fileErr = "";
try { qrCode.from( "x" ).file(); } catch ( any e ) { fileErr = e.message; }
assertTrue( "file() is refused and names the alternative", findNoCase( "toByteArray", fileErr ) > 0 );

// ---- Batik ----------------------------------------------------------------
svg = '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40" viewBox="0 0 80 40">'
    & '<rect width="80" height="40" fill="##0000ff"/></svg>';
svgPath = getTempDirectory() & "/rustcfml_batik_" & createUUID() & ".svg";
pngPath = getTempDirectory() & "/rustcfml_batik_" & createUUID() & ".png";
fileWrite( svgPath, svg );

// Preside's exact shape, including the file: URI from java.io.File.toURL().
t       = CreateObject( "java", "org.apache.batik.transcoder.image.PNGTranscoder", [ "/no/such/batik.jar" ] ).init();
svgURI  = CreateObject( "java", "java.io.File" ).init( svgPath ).toURL().toString();
input   = CreateObject( "java", "org.apache.batik.transcoder.TranscoderInput", [ "" ] ).init( svgURI );
ostream = CreateObject( "java", "java.io.FileOutputStream" ).init( pngPath );
output  = CreateObject( "java", "org.apache.batik.transcoder.TranscoderOutput", [ "" ] ).init( ostream );

assertTrue( "the file: URI from File.toURL() resolves", findNoCase( "file:", svgURI ) == 1 );

// KEY_WIDTH / KEY_HEIGHT are public static FIELDS, read off the transcoder.
assert( "KEY_WIDTH is readable as a field", t.KEY_WIDTH, "KEY_WIDTH" );
t.addTranscodingHint( t.KEY_WIDTH, javaCast( "float", 320 ) );
t.transcode( input, output );
ostream.flush();
ostream.close();

assertTrue( "transcode produced a file", fileExists( pngPath ) );
assert( "as a PNG", ucase( left( binaryEncode( fileReadBinary( pngPath ), "hex" ), 8 ) ), "89504E47" );
assert( "at the hinted width", imageInfo( imageRead( pngPath ) ).width, 320 );
assert( "with the aspect ratio kept", imageInfo( imageRead( pngPath ) ).height, 160 );
fileDelete( pngPath );

// Hints accumulate, and nothing renders until transcode().
png2 = getTempDirectory() & "/rustcfml_batik_" & createUUID() & ".png";
t2 = CreateObject( "java", "org.apache.batik.transcoder.image.PNGTranscoder" ).init();
t2.addTranscodingHint( t2.KEY_WIDTH, javaCast( "float", 200 ) );
t2.addTranscodingHint( t2.KEY_HEIGHT, javaCast( "float", 200 ) );
os2 = CreateObject( "java", "java.io.FileOutputStream" ).init( png2 );
t2.transcode(
	  CreateObject( "java", "org.apache.batik.transcoder.TranscoderInput" ).init( svgPath )
	, CreateObject( "java", "org.apache.batik.transcoder.TranscoderOutput" ).init( os2 )
);
assert( "both hints are honoured", imageInfo( imageRead( png2 ) ).width, 200 );
assert( "…as a fitted box", imageInfo( imageRead( png2 ) ).height, 200 );
fileDelete( png2 );

// A broken SVG raises the type Preside's catch expects, rather than writing
// an empty file and reporting success.
badSvg = getTempDirectory() & "/rustcfml_batik_bad_" & createUUID() & ".svg";
fileWrite( badSvg, "not svg at all" );
badType = "";
try {
	t3  = CreateObject( "java", "org.apache.batik.transcoder.image.PNGTranscoder" ).init();
	os3 = CreateObject( "java", "java.io.FileOutputStream" ).init( getTempDirectory() & "/rustcfml_batik_bad.png" );
	t3.transcode(
		  CreateObject( "java", "org.apache.batik.transcoder.TranscoderInput" ).init( badSvg )
		, CreateObject( "java", "org.apache.batik.transcoder.TranscoderOutput" ).init( os3 )
	);
} catch ( any e ) {
	badType = e.type;
}
assert( "a broken SVG raises TranscoderException", badType, "org.apache.batik.transcoder.TranscoderException" );
fileDelete( badSvg );
if ( fileExists( getTempDirectory() & "/rustcfml_batik_bad.png" ) ) {
	fileDelete( getTempDirectory() & "/rustcfml_batik_bad.png" );
}
fileDelete( svgPath );

suiteEnd();
</cfscript>
