<cfscript>
// Apache Commons Imaging shim — Preside's JavaImageMetaReader.readMeta() reads
// image metadata via `org.apache.commons.imaging.Imaging.getImageInfo(file)`.
// Without a JVM this class was unsupported, readMeta caught the error and
// returned {}, and every Preside asset upload failed "Unrecognized image
// format". These tests exercise the shim end to end.
suiteBegin( "Java shim: Apache Commons Imaging (Preside image metadata)" );

// Build a real PNG on disk to read back (no fixture file needed).
tmpPng = getTempDirectory() & "/rustcfml_commons_imaging_test.png";
img = imageNew( "", 120, 80, "rgb", "ff9900" );
imageWrite( img, tmpPng );

// The exact call chain Preside's JavaImageMetaReader.readMeta uses.
fileobj   = createObject( "java", "java.io.File" ).init( tmpPng );
imaging   = createObject( "java", "org.apache.commons.imaging.Imaging" );
imageInfo = imaging.getImageInfo( fileobj );

assert( "getWidth reports the pixel width",  imageInfo.getWidth(),  120 );
assert( "getHeight reports the pixel height", imageInfo.getHeight(), 80 );
assert( "getFormatName detects PNG from magic bytes", imageInfo.getFormatName(), "PNG" );
assertTrue( "getNumberOfImages is at least 1", imageInfo.getNumberOfImages() >= 1 );
assertFalse( "a baseline PNG is not progressive", imageInfo.isProgressive() );
assertTrue( "bitsPerPixel is populated", isNumeric( imageInfo.getBitsPerPixel() ) && imageInfo.getBitsPerPixel() > 0 );

// isValidImageFile's actual gate: readMeta returns a struct that HAS a height key.
meta = {
      width  = imageInfo.getWidth()
    , height = imageInfo.getHeight()
    , format = imageInfo.getFormatName()
};
assertTrue( "meta struct carries a height key (drives isValidImageFile)", structKeyExists( meta, "height" ) );

// A path String (not just a File) is also accepted.
infoFromPath = isRustCFML() ? imaging.getImageInfo( tmpPng ) : "";
// Real Imaging.getImageInfo takes a File; accepting a path string is a
// RustCFML convenience, so Lucee has no matching method.
if ( isRustCFML() ) {
    assert( "getImageInfo accepts a path string too", infoFromPath.getWidth(), 120 );
}

// Non-image bytes must throw (Apache getImageInfo throws; readMeta catches it).
threw = false;
try {
    bogus = getTempDirectory() & "/rustcfml_commons_imaging_notanimage.txt";
    fileWrite( bogus, "this is definitely not an image" );
    imaging.getImageInfo( createObject( "java", "java.io.File" ).init( bogus ) );
} catch ( any e ) {
    threw = true;
}
assertTrue( "getImageInfo throws on non-image bytes", threw );

suiteEnd();
</cfscript>
