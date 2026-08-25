<cfscript>
suiteBegin("PDF reading and page rasterisation");

// CFML's PDF story has always been a Java one. This is native, backed by hayro
// — the renderer Typst uses for embedded PDFs, tested against 1400+ files from
// the PDFBOX and pdf.js regression corpora.
//
// The fixture is a hand-built 200x100pt PDF containing the text "RustCFML PDF"
// and a blue rectangle, committed so these tests need nothing external.

// Anchored on this file's own directory: the runner includes it, and the two
// engines disagree on what a web-root-absolute "/tests/..." resolves to.
fixture = getDirectoryFromPath( getCurrentTemplatePath() ) & "fixtures/sample.pdf";

doc = pdfRead( fixture );
assertTrue( "pdfRead returns a PDF object", isPdfObject( doc ) );
assertFalse( "and a plain value is not one", isPdfObject( "x" ) );
assert( "pdfPageCount reports the pages", pdfPageCount( doc ), 1 );

info = pdfInfo( doc );
assert( "info reports the page count", info.pages, 1 );
// Page sizes are in PostScript points (1/72"), which is what a PDF measures in.
assert( "…and the page size in points", info.pagesizes[ 1 ].width, 200 );
assert( "…on both edges", info.pagesizes[ 1 ].height, 100 );

// A rendered page is an ordinary IMAGE OBJECT, so the whole image family
// applies. That is what makes this compose rather than being a converter.
img = pdfToImage( doc, 1, 400 );
assert( "a page renders at the requested width", imageInfo( img ).width, 400 );
assert( "…keeping the page's aspect ratio", imageInfo( img ).height, 200 );

// Without a width, dpi decides. A PDF's native resolution is 72dpi, so 144dpi
// doubles a 200pt page to 400px.
assert( "dpi scales from the PDF's native 72", imageInfo( pdfToImage( doc, 1, 0, 144 ) ).width, 400 );
assert( "and with neither, it renders 1:1 in points", imageInfo( pdfToImage( doc, 1 ) ).width, 200 );

// Fluent form: a PDF is read-only, so every method is terminal.
assert( "Pdf() chains", imageInfo( Pdf( fixture ).toImage( 1, 300 ) ).width, 300 );
assert( "…and exposes pageCount", Pdf( fixture ).pageCount(), 1 );

// Binary input, not just a path.
assert( "a PDF can be read from binary", pdfPageCount( pdfRead( fileReadBinary( fixture ) ) ), 1 );

// It really rendered the content, rather than handing back a blank canvas: the
// fixture has black text and a blue bar on white.
out = getTempDirectory() & "/rustcfml_pdf_" & createUUID() & ".png";
imageWrite( pdfToImage( doc, 1, 400 ), out );
assertTrue( "the rendered page writes out as a raster image", fileExists( out ) );
assertTrue( "…and carries real ink, not a blank page", getFileInfo( out ).size > 1000 );
fileDelete( out );

// Errors name the problem.
assertThrows( "a page beyond the end is refused", function(){ pdfToImage( doc, 99 ); } );
assertThrows( "page 0 is refused (pages are 1-based)", function(){ pdfToImage( doc, 0 ); } );
assertThrows( "a missing file is refused", function(){ pdfRead( "/no/such/file.pdf" ); } );
assertThrows( "a non-PDF is refused", function(){ pdfRead( getCurrentTemplatePath() ); } );
assertThrows( "a non-PDF first argument is refused", function(){ pdfInfo( "not a pdf" ); } );

suiteEnd();
</cfscript>
