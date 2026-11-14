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

// ZXing's QRCodeWriter renders exactly width x height: the square symbol is
// centred, the rest is white padding (GH #518).
oblong = qrCode.from( "x" ).to( imageTypes.PNG ).withSize( 200, 80 ).stream().toByteArray();
assert( "a non-square request keeps its width", imageInfo( imageNew( oblong ) ).width, 200 );
assert( "and its height", imageInfo( imageNew( oblong ) ).height, 80 );

// GH #518: the same BYTES as QRGen 2.2.0 / ZXing 3.2.1 on the JVM. Every MD5
// below was captured from the real jars on Lucee 7.1 (Java ImageIO's GIF
// writer). The first two are Preside's own QrCodeGeneratorTest fixtures
// (helloWorldDefaults.gif, helloWorld300x300.gif). A different EC default
// (it used to be M; ZXing's is L), a different mask choice, stretching rather
// than padding, or a different GIF encoder each changes these.
function qrgenMd5( text, w, h, ec = "", cs = "" ) {
	var q = qrCode.from( arguments.text ).to( imageTypes.GIF ).withSize( arguments.w, arguments.h );
	if ( len( arguments.ec ) ) q = q.withErrorCorrection( arguments.ec );
	if ( len( arguments.cs ) ) q = q.withCharset( arguments.cs );
	return lcase( hash( q.stream().toByteArray(), "MD5" ) );
}
otp = "otpauth://totp/Preside:sysadmin?secret=GEZDGNBVGY3TQOJQ&issuer=Preside";
assert( "hello world 125x125 is Preside's helloWorldDefaults.gif", qrgenMd5( "hello world", 125, 125 ), "aaafbfc299e51284c8554b0e38a58679" );
assert( "hello world 300x300 is Preside's helloWorld300x300.gif",  qrgenMd5( "hello world", 300, 300 ), "39bfda8fe3f14635bb2ca63b4f76f47a" );
assert( "a non-square request is padded, as ZXing pads it",       qrgenMd5( "hello world", 201, 150 ), "155558deb91ad55b001215374ca7ea2a" );
assert( "a TOTP enrolment URL (byte mode, version 4)",            qrgenMd5( otp, 125, 125 ),           "77bf85f6b3266430fd7faa48b9c1d1ec" );
assert( "withErrorCorrection( H ) at an uneven size",             qrgenMd5( otp, 250, 250, "H" ),      "f293e71fce0fcbb2f07b9527364eb2e4" );
assert( "withCharset( UTF-8 ) writes an ECI header",              qrgenMd5( "café", 125, 125, "", "UTF-8" ), "c11891cc0060a579de71f9ad83b3238d" );
assert( "alphanumeric mode at an odd size",                       qrgenMd5( "HELLO WORLD", 157, 157 ), "9da013a4f99872ae11459a445cf8a54f" );
assert( "numeric mode, smaller than the symbol: grows to fit",    qrgenMd5( "0123456789012345", 10, 10 ), "f43e8c444ed8e138e00f339a4b0c4a0c" );

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
