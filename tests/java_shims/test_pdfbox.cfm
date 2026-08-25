<cfscript>
suiteBegin("java shim: org.apache.pdfbox over the Pdf* builtins");

// Preside's NativeImageService rasterises the first page of an uploaded PDF so
// the asset pipeline has a thumbnail. It drives PDFBox 1.x's PDFImageWriter,
// whose contract is to write one file per page named <prefix><n>.<format>.

fixture = getDirectoryFromPath( getCurrentTemplatePath() ) & "../stdlib/fixtures/sample.pdf";
prefix  = getTempDirectory() & "/rustcfml_pdfbox_" & left( createUUID(), 8 );

bufferedImage = createObject( "java", "java.awt.image.BufferedImage" );
imageWriter   = createObject( "java", "org.apache.pdfbox.util.PDFImageWriter" );
document      = createObject( "java", "org.apache.pdfbox.pdmodel.PDDocument" ).load( fixture );

assert( "PDDocument.load reads the document", document.getNumberOfPages(), 1 );
// The TYPE_* constants are public static FIELDS, read off the instance.
assert( "BufferedImage.TYPE_INT_RGB is readable as a field", bufferedImage.TYPE_INT_RGB, 1 );
assert( "…and TYPE_INT_ARGB", bufferedImage.TYPE_INT_ARGB, 2 );

// The 8th argument is a RESOLUTION (DPI), as PDFBox defines it — not a width.
// The fixture is 200pt wide, so 144dpi gives 400px.
imageWriter.writeImage( document, javaCast( "string", "jpg" ), javaCast( "string", "" ), "1", "1",
                        javaCast( "string", prefix ), bufferedImage.TYPE_INT_RGB, 144 );
document.close();

written = prefix & "1.jpg";
assertTrue( "writeImage honours PDFBox's <prefix><page>.<format> naming", fileExists( written ) );
assert( "…and the resolution argument is DPI, not pixels", imageInfo( imageRead( written ) ).width, 400 );
fileDelete( written );

// PDFBox 2.x's PDFRenderer, whose page indices are 0-BASED.
doc2 = createObject( "java", "org.apache.pdfbox.pdmodel.PDDocument" ).load( fixture );
renderer = createObject( "java", "org.apache.pdfbox.rendering.PDFRenderer" ).init( doc2 );
assert( "renderImageWithDPI( 0, 144 ) renders the FIRST page"
      , imageInfo( renderer.renderImageWithDPI( 0, 144 ) ).width, 400 );
// renderImage's second argument is a SCALE factor, not a DPI.
assert( "renderImage( 0, 2 ) doubles the page", imageInfo( renderer.renderImage( 0, 2 ) ).width, 400 );

// Reading a document is supported; authoring one is not, and says so.
authorErr = "";
try { doc2.save( "/tmp/x.pdf" ); } catch ( any e ) { authorErr = e.message; }
assertTrue( "PDF authoring is refused, not faked", findNoCase( "not supported", authorErr ) > 0 );

// A missing file surfaces as the IOException PDFBox callers catch.
loadErr = "";
try { createObject( "java", "org.apache.pdfbox.pdmodel.PDDocument" ).load( "/no/such.pdf" ); }
catch ( any e ) { loadErr = e.type; }
assert( "a missing PDF raises IOException", loadErr, "java.io.IOException" );

suiteEnd();
</cfscript>
