<cfscript>
// GH #389. ImageGetEXIFMetadata() returned only the raw EXIF IFD tags. Lucee
// merges the image's BASELINE info keys into the same struct — width, height,
// source, colormodel, jpeg_color_type, metadata — whichever way the image was
// read, so `meta.width` was absent here while ImageInfo() on the same image
// reported it. Preside's DocumentMetadataService appends this struct and
// expects width/height for every uploaded image.
//
// Measured against Lucee 7.1.0.204 with the fixture below: an 8x8 JPEG
// carrying a hand-built EXIF APP1 segment (XResolution, YResolution,
// ResolutionUnit, YCbCrPositioning, Copyright) alongside an IPTC APP13.
suiteBegin("Image EXIF baseline keys");

exifJpegB64 = "/9j/4QBxRXhpZgAATU0AKgAAAAgABQEaAAUAAAABAAAASgEbAAUAAAABAAAAUgEoAAMAAAABAAIAAAITAAMAAAABAAEAAIKYAAIAAAAPAAAAWgAAAAAAAABIAAAAAQAAAEgAAAABVGVzdCBDb3B5cmlnaHQA/+0AXFBob3Rvc2hvcCAzLjAAOEJJTQQEAAAAAABAHAIFAAhNeSBUaXRsZRwCGQAFYWxwaGEcAhkABGJldGEcAngADkEgY2FwdGlvbiBoZXJlHAJQAAhKYW5lIERvZf/gABBKRklGAAECAAABAAEAAP/AABEIAAgACAMBEQACEQEDEQH/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/xAAfAAABBQEBAQEBAQAAAAAAAAAAAQIDBAUGBwgJCgv/xAC1EAACAQMDAgQDBQUEBAAAAX0BAgMABBEFEiExQQYTUWEHInEUMoGRoQgjQrHBFVLR8CQzYnKCCQoWFxgZGiUmJygpKjQ1Njc4OTpDREVGR0hJSlNUVVZXWFlaY2RlZmdoaWpzdHV2d3h5eoOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4eLj5OXm5+jp6vHy8/T19vf4+fr/xAAfAQADAQEBAQEBAQEBAAAAAAAAAQIDBAUGBwgJCgv/xAC1EQACAQIEBAMEBwUEBAABAncAAQIDEQQFITEGEkFRB2FxEyIygQgUQpGhscEJIzNS8BVictEKFiQ04SXxFxgZGiYnKCkqNTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqCg4SFhoeIiYqSk5SVlpeYmZqio6Slpqeoqaqys7S1tre4ubrCw8TFxsfIycrS09TV1tfY2dri4+Tl5ufo6ery8/T19vf4+fr/2gAMAwEAAhEDEQA/AOLr5k/cT//Z";
exifBin = toBinary( exifJpegB64 );

// ---- base64-read: the form Preside uses, and the minimal contract ----------
b = imageGetEXIFMetadata( imageReadBase64( exifJpegB64 ) );
assertTrue( "returns a struct", isStruct( b ) );
assert( "width is the image width", b.width, 8 );
assert( "height is the image height", b.height, 8 );
assertTrue( "width is numeric", isNumeric( b.width ) );
assertTrue( "source key is present", structKeyExists( b, "source" ) );
assert( "a base64-read image has no source file", len( b.source ), 0 );
assertTrue( "colormodel is a struct", isStruct( b.colormodel ) );
assert( "jpeg_color_type", b.jpeg_color_type, "RGB" );
assertTrue( "metadata is a struct", isStruct( b.metadata ) );

// ---- file-read: same baseline keys, plus the raw tags ----------------------
// Written to a temp file so the fixture travels with the test.
tmpDir = getTempDirectory();
exifPath = tmpDir & "rustcfml_exif_probe.jpg";
fileWrite( exifPath, exifBin );

a = imageGetEXIFMetadata( imageRead( exifPath ) );
assert( "file-read width", a.width, 8 );
assert( "file-read height", a.height, 8 );
assertTrue( "file-read reports its source", len( a.source ) GT 0 );
assertTrue( "file-read colormodel is a struct", isStruct( a.colormodel ) );

// Tag VALUES are the RAW ones, not display strings: Lucee reports 72, not
// "72 pixels per inch", and 2, not "inch". A numeric tag must be numeric.
assert( "XResolution is the raw value", a.XResolution, 72 );
assertTrue( "so it is numeric", isNumeric( a.XResolution ) );
assert( "YResolution is the raw value", a.YResolution, 72 );
assert( "ResolutionUnit is the raw enum value", a.ResolutionUnit, 2 );
assert( "YCbCrPositioning is the raw enum value", a.YCbCrPositioning, 1 );
// An ASCII tag is unquoted.
assert( "Copyright has no surrounding quotes", a.Copyright, "Test Copyright" );

// The exif / gps sub-structs Lucee carries on the file-read path.
assertTrue( "exif sub-struct is present", isStruct( a.exif ) );
assert( "carrying the five tags", structCount( a.exif ), 5 );
assertTrue( "gps sub-struct is present", isStruct( a.gps ) );
assert( "empty for an image with no GPS IFD", structCount( a.gps ), 0 );

// imageGetEXIFTag must agree with the struct.
assert( "getEXIFTag agrees on a string tag",
        imageGetEXIFTag( imageRead( exifPath ), "Copyright" ), "Test Copyright" );
assert( "getEXIFTag agrees on a numeric tag",
        imageGetEXIFTag( imageRead( exifPath ), "XResolution" ), 72 );

// ---- imageInfo carries the same baseline set ------------------------------
i = imageInfo( imageRead( exifPath ) );
assert( "imageInfo width", i.width, 8 );
assertTrue( "imageInfo reports jpeg_color_type", structKeyExists( i, "jpeg_color_type" ) );
// NOT asserted: `metadata` on imageInfo of a FILE-read image. Lucee's own
// split is driven by which reader it could run — a file-read image gets
// `exif`/`gps` and no `metadata`, a base64-read one the reverse. We emit all
// three on both paths, which is a superset, so this is deliberately untested
// across engines rather than pinned to our own behaviour.

fileDelete( exifPath );
suiteEnd();
</cfscript>
