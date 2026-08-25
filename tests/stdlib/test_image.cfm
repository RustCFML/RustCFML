<cfscript>
suiteBegin("Image functions");

// ============================================================
// Image support — imageNew/imageRead/imageResize/... and <cfimage>.
//
// Behaviour here is pinned to Lucee's image extension
// (github.com/lucee/extension-image): the imageInfo() colormodel key names and
// colorspace/transparency strings, the resize aspect-ratio rule, the
// blurFactor 0..10 / quality 0..1 bounds, content-based format detection, and
// base64 round-tripping.
//
// Fixtures are generated in-memory (imageNew) and round-tripped through the
// temp directory / base64, so the suite is self-contained (no binary assets).
// NOTE: Lucee ships image as an OPTIONAL extension; if it is not installed the
// server has no image functions and this suite is not comparable there. On
// RustCFML the functions are always built in (image_support feature).
// ============================================================

tmpDir = getTempDirectory();

// ---- imageNew: blank canvas + dimensions --------------------------------
img = imageNew("", 200, 100, "rgb", "red");
assertTrue("isImage(blank canvas)", isImage(img));
assert("imageGetWidth", imageGetWidth(img), 200);
assert("imageGetHeight", imageGetHeight(img), 100);
assert("member getWidth", img.getWidth(), 200);
assert("member getHeight", img.getHeight(), 100);

// imageNew with width+height requires an empty source
assertThrows("imageNew: source + dimensions is illegal", function() {
    imageNew("some/path.png", 100, 100);
});

// invalid imageType
assertThrows("imageNew: invalid imageType", function() {
    imageNew("", 10, 10, "cmyk");
});

// grayscale + argb types construct
assertTrue("imageNew grayscale", isImage(imageNew("", 10, 10, "grayscale")));
assertTrue("imageNew argb", isImage(imageNew("", 10, 10, "argb")));

// ---- imageInfo: struct shape (Lucee key names) --------------------------
info = imageInfo(img);
assert("info.width", info.width, 200);
assert("info.height", info.height, 100);
assertTrue("info has colormodel", structKeyExists(info, "colormodel"));
assert("colormodel.colorspace RGB",
    info.colormodel.colorspace, "Any of the family of RGB color spaces");
assert("colormodel.transparency opaque (rgb)", info.colormodel.transparency, "OPAQUE");
assert("colormodel.num_color_components", info.colormodel.num_color_components, 3);
assert("colormodel.pixel_size (3x8)", info.colormodel.pixel_size, 24);
assertTrue("colormodel.bits_component is array", isArray(info.colormodel.bits_component));
assert("colormodel.bits_component len", arrayLen(info.colormodel.bits_component), 3);
assert("colormodel.bits_component_1", info.colormodel.bits_component_1, 8);
assertFalse("colormodel.alpha_channel_support (rgb)", info.colormodel.alpha_channel_support);
// in-memory image has an empty source
assert("info.source empty for in-memory", len(info.source), 0);
// member form of info() is identical
assert("member info().width", img.info().width, 200);

// argb reports alpha + TRANSLUCENT
argbInfo = imageInfo(imageNew("", 5, 5, "argb"));
assertTrue("argb alpha_channel_support", argbInfo.colormodel.alpha_channel_support);
assert("argb transparency", argbInfo.colormodel.transparency, "TRANSLUCENT");

// grayscale colorspace
grayInfo = imageInfo(imageNew("", 5, 5, "grayscale"));
assert("grayscale colorspace",
    grayInfo.colormodel.colorspace, "Any of the family of GRAY color spaces");

// ---- imageResize: exact, aspect-preserving, percentage ------------------
r1 = imageNew("", 200, 100, "rgb", "blue");
imageResize(r1, 92, 46);
assert("resize exact width", imageGetWidth(r1), 92);
assert("resize exact height", imageGetHeight(r1), 46);

// width only → height preserved by aspect ratio (200x100 -> 50 wide -> 25 tall)
r2 = imageNew("", 200, 100, "rgb", "blue");
imageResize(r2, 50, "");
assert("resize width-only preserves aspect W", imageGetWidth(r2), 50);
assert("resize width-only preserves aspect H", imageGetHeight(r2), 25);

// height only → width preserved
r3 = imageNew("", 200, 100, "rgb", "blue");
imageResize(r3, "", 25);
assert("resize height-only preserves aspect W", imageGetWidth(r3), 50);
assert("resize height-only preserves aspect H", imageGetHeight(r3), 25);

// Lucee's own ImageResize.cfc case: 92x59 doubled by width preserves 59*2=118
r4 = imageNew("", 92, 59, "rgb", "blue");
r4.resize(184, "");
assert("resize 92x59 -> 184 wide", imageGetWidth(r4), 184);
assert("resize 92x59 -> 118 tall", imageGetHeight(r4), 118);

// percentage
r5 = imageNew("", 200, 100, "rgb", "blue");
imageResize(r5, "50%", "50%");
assert("resize 50% width", imageGetWidth(r5), 100);
assert("resize 50% height", imageGetHeight(r5), 50);

// interpolation names must not throw
imageResize(imageNew("", 40, 40, "rgb", "red"), 20, 20, "nearest");
imageResize(imageNew("", 40, 40, "rgb", "red"), 20, 20, "bilinear");
imageResize(imageNew("", 40, 40, "rgb", "red"), 20, 20, "bicubic");
imageResize(imageNew("", 40, 40, "rgb", "red"), 20, 20, "highestQuality");
assertTrue("interpolation names accepted", true);

// blurFactor bounds (Lucee: 0..10 inclusive)
assertThrows("blurFactor 11 rejected", function() {
    imageResize(imageNew("", 40, 40, "rgb", "red"), 20, 20, "highestQuality", 11);
});
assertThrows("blurFactor -1 rejected", function() {
    imageResize(imageNew("", 40, 40, "rgb", "red"), 20, 20, "highestQuality", -1);
});

// negative/zero dimension rejected
assertThrows("resize to 0 rejected", function() {
    imageResize(imageNew("", 40, 40, "rgb", "red"), 0, 0);
});

// ---- imageScaleToFit -----------------------------------------------------
s1 = imageNew("", 200, 100, "rgb", "green");
imageScaleToFit(s1, 100, 100);
assertTrue("scaleToFit fits within box (W<=100)", imageGetWidth(s1) <= 100);
assertTrue("scaleToFit fits within box (H<=100)", imageGetHeight(s1) <= 100);
assert("scaleToFit preserves aspect (200x100 -> 100x50)", imageGetHeight(s1), 50);

// ---- imageCrop -----------------------------------------------------------
c1 = imageNew("", 100, 80, "rgb", "red");
imageCrop(c1, 10, 10, 40, 30);
assert("crop width", imageGetWidth(c1), 40);
assert("crop height", imageGetHeight(c1), 30);

// ---- imageRotate (quarter turns) ----------------------------------------
rot = imageNew("", 100, 40, "rgb", "red");
imageRotate(rot, 90);
assert("rotate 90 swaps W", imageGetWidth(rot), 40);
assert("rotate 90 swaps H", imageGetHeight(rot), 100);

// ---- imageFlip -----------------------------------------------------------
fl = imageNew("", 60, 30, "rgb", "red");
imageFlip(fl, "horizontal");
assert("flip keeps dims (horizontal)", imageGetWidth(fl) & "x" & imageGetHeight(fl), "60x30");
fl2 = imageNew("", 60, 30, "rgb", "red");
imageFlip(fl2, "90");
assert("flip 90 swaps dims", imageGetWidth(fl2) & "x" & imageGetHeight(fl2), "30x60");

// ---- imageWrite / imageRead round-trip (content-based detection) --------
pngPath = tmpDir & "/rustcfml_img_test.png";
imageWrite(img, pngPath);
back = imageRead(pngPath);
assert("write/read round-trip W", imageGetWidth(back), 200);
assert("write/read round-trip H", imageGetHeight(back), 100);
assertTrue("read-from-file source is non-empty", len(imageInfo(back).source) > 0);

// Cross-format: write PNG image to JPEG then read back
jpgPath = tmpDir & "/rustcfml_img_test.jpg";
imageWrite(img, jpgPath, 0.8);
backJpg = imageRead(jpgPath);
assert("cross-format jpg round-trip W", imageGetWidth(backJpg), 200);

// Content-based detection: a JPEG written to a .png name still reads
misnamed = tmpDir & "/rustcfml_actually_jpg.png";
imageWrite(img, jpgPath, 0.8);
fileCopy(jpgPath, misnamed);
misImg = imageRead(misnamed);
assert("misnamed file read by content", imageGetWidth(misImg), 200);

// quality must be 0..1
assertThrows("quality > 1 rejected", function() {
    imageWrite(img, tmpDir & "/rustcfml_bad_quality.jpg", 2);
});

// imageRead of a non-existent file throws
assertThrows("imageRead missing file throws", function() {
    imageRead(tmpDir & "/rustcfml_does_not_exist_zzz.png");
});

// ---- imageGetBlob --------------------------------------------------------
blob = imageGetBlob(img);
assertTrue("getBlob returns binary", isBinary(blob));
assertTrue("getBlob has bytes", len(toBase64(blob)) > 0);

// ---- base64 round-trip ---------------------------------------------------
b64 = imageWriteBase64(img, "", "png");
assertTrue("writeBase64 returns non-empty string", len(b64) > 0);
fromB64 = imageReadBase64(b64);
assert("base64 round-trip W", imageGetWidth(fromB64), 200);
assert("base64 round-trip H", imageGetHeight(fromB64), 100);

// data: URI form
htmlB64 = imageWriteBase64(img, "", "png", true);
assertTrue("writeBase64 inHTMLFormat has data URI prefix", left(htmlB64, 5) == "data:");

// ---- isImage / isImageFile / format lists -------------------------------
assertFalse("isImage(string)", isImage("not an image"));
assertFalse("isImage(number)", isImage(42));
assertTrue("isImageFile(png)", isImageFile(pngPath));
assertFalse("isImageFile(missing)", isImageFile(tmpDir & "/nope_zzz.png"));
assertTrue("getReadableImageFormats non-empty", len(getReadableImageFormats()) > 0);
assertTrue("PNG readable", listFindNoCase(getReadableImageFormats(), "PNG") > 0);

// ---- <cfimage> tag -------------------------------------------------------
</cfscript>
<cfimage action="read" source="#pngPath#" name="tagRead">
<cfscript>
assert("cfimage read W", imageGetWidth(tagRead), 200);
</cfscript>
<cfimage action="resize" source="#pngPath#" name="tagResize" width="40" height="20">
<cfscript>
assert("cfimage resize W", imageGetWidth(tagResize), 40);
assert("cfimage resize H", imageGetHeight(tagResize), 20);
</cfscript>
<cfimage action="info" source="#pngPath#" structName="tagInfo">
<cfscript>
assert("cfimage info W", tagInfo.width, 200);
assert("cfimage info colorspace", tagInfo.colormodel.colorspace, "Any of the family of RGB color spaces");
</cfscript>
<cfimage action="write" source="#img#" destination="#tmpDir#/rustcfml_cfimage_write.png" overwrite="true">
<cfscript>
assertTrue("cfimage write produced file", fileExists(tmpDir & "/rustcfml_cfimage_write.png"));

// GitHub #253: path auto-cast straight into the info BIFs, and the script-call
// form of <cfimage> honouring structName as a write-back (Wheels imageTag probe).
imageWrite(imageNew("", 12, 34, "rgb", "red"), tmpDir & "/rustcfml_probe.png");
assert("path auto-cast into imageGetWidth", imageGetWidth(tmpDir & "/rustcfml_probe.png"), 12);
assert("path auto-cast into imageGetHeight", imageGetHeight(tmpDir & "/rustcfml_probe.png"), 34);
assert("path auto-cast into imageInfo", imageInfo(tmpDir & "/rustcfml_probe.png").width, 12);

cfimage(attributeCollection = {action: "info", source: tmpDir & "/rustcfml_probe.png", structName: "scriptRv"});
assertTrue("cfimage script-call form defines structName target", isDefined("scriptRv"));
assert("cfimage script-call form info width", scriptRv.width, 12);
if (fileExists(tmpDir & "/rustcfml_probe.png")) { fileDelete(tmpDir & "/rustcfml_probe.png"); }

// ============================================================
// Tier 2 — drawing (state + primitives + text + compositing).
//
// There is no CFML pixel-accessor, and Java2D-vs-imageproc rasterisation is not
// pixel-identical, so these assert structural invariants: the ops don't throw,
// dimensions are preserved (or change as specified), and the result stays a
// valid, encodable image. See docs/known-issues.md §18.
// ============================================================
canvas = imageNew("", 100, 60, "rgb", "white");
imageSetDrawingColor(canvas, "blue");
imageSetBackgroundColor(canvas, "white");
imageSetDrawingStroke(canvas, {width: 3});
imageSetDrawingTransparency(canvas, 0);
imageDrawLine(canvas, 0, 0, 100, 60);
imageDrawRect(canvas, 5, 5, 30, 20, false);
imageDrawRect(canvas, 40, 5, 30, 20, true);
imageDrawOval(canvas, 60, 30, 20, 20, false);
imageDrawRoundRect(canvas, 5, 30, 30, 20, 8, 8, false);
imageDrawBeveledRect(canvas, 40, 30, 30, 20, true, true);
imageDrawArc(canvas, 75, 5, 20, 20, 0, 90, false);
imageDrawPoint(canvas, 50, 50);
imageDrawCubicCurve(canvas, 0, 0, 20, 60, 80, 0, 100, 60);
imageDrawQuadraticCurve(canvas, 0, 30, 50, 0, 100, 30);
imageDrawLines(canvas, [0, 50, 100], [0, 30, 0], true, false);
imageSetAntialiasing(canvas, "on");
imageDrawLine(canvas, 0, 60, 100, 0);
imageDrawText(canvas, "Test", 10, 40, {size: 12});
imageClearRect(canvas, 90, 50, 10, 10);
assertTrue("drawing kept a valid image", isImage(canvas));
assert("drawing preserved width", imageGetWidth(canvas), 100);
assert("drawing preserved height", imageGetHeight(canvas), 60);
assertTrue("drawn image still encodes", len(imageWriteBase64(canvas, "", "png")) GT 0);

// filled polygon via drawLines(isPolygon, filled)
poly = imageNew("", 40, 40, "rgb", "white");
imageSetDrawingColor(poly, "green");
imageDrawLines(poly, [5, 35, 20], [35, 35, 5], true, true);
assertTrue("filled polygon is image", isImage(poly));

// ---- compositing --------------------------------------------------------
base = imageNew("", 50, 50, "rgb", "black");
imagePaste(base, imageNew("", 20, 20, "rgb", "red"), 5, 5);
assert("paste preserves base width", imageGetWidth(base), 50);
imageDrawImage(base, imageNew("", 10, 10, "rgb", "blue"), 30, 30);
assert("drawImage preserves base height", imageGetHeight(base), 50);
imageOverlay(base, imageNew("", 50, 50, "argb"), "over", 0.5);
assert("overlay preserves dimensions", imageGetWidth(base), 50);
imageCopy(base, 0, 0, 10, 10, 20, 20);
assert("copy preserves dimensions", imageGetHeight(base), 50);

bordered = imageNew("", 30, 30, "rgb", "white");
imageAddBorder(bordered, 4, "black");
assert("addBorder grows width by 2*thickness", imageGetWidth(bordered), 38);
assert("addBorder grows height by 2*thickness", imageGetHeight(bordered), 38);

// <cfimage action="border"> writes back through name=
cfimage(action = "border", source = imageNew("", 20, 20, "rgb", "white"), thickness = 3, color = "black", name = "cfBordered");
assert("cfimage border grows width", imageGetWidth(cfBordered), 26);

// <cfimage action="captcha">
cfimage(action = "captcha", text = "RustCFML", width = 180, height = 44, name = "captchaImg");
assertTrue("captcha produced an image", isImage(captchaImg));
assert("captcha honours width", imageGetWidth(captchaImg), 180);
assert("captcha honours height", imageGetHeight(captchaImg), 44);

// ============================================================
// Tier 3 — filters + transforms + metadata.
// ============================================================
fx = imageNew("", 40, 40, "rgb", "128,64,200");
imageBlur(fx, 2);
assert("blur preserves width", imageGetWidth(fx), 40);
imageSharpen(fx, 1);
assert("sharpen preserves height", imageGetHeight(fx), 40);
imageNegative(fx);
assertTrue("negative kept an image", isImage(fx));
imageGrayscale(fx);
assertTrue("grayscale kept an image", isImage(fx));

// makeTranslucent / makeColorTransparent flip on the alpha channel
translucent = imageNew("", 30, 30, "rgb", "red");
imageMakeTranslucent(translucent, 50);
assertTrue("makeTranslucent introduces an alpha channel",
    imageInfo(translucent).colormodel.alpha_channel_support);
keyed = imageNew("", 30, 30, "rgb", "red");
imageMakeColorTransparent(keyed, "red");
assertTrue("makeColorTransparent introduces an alpha channel",
    imageInfo(keyed).colormodel.alpha_channel_support);

// translate / shear keep the canvas size; arbitrary-angle rotate grows it
moved = imageNew("", 30, 30, "rgb", "red");
imageTranslate(moved, 5, 5);
assert("translate preserves width", imageGetWidth(moved), 30);
sheared = imageNew("", 30, 30, "rgb", "red");
imageShear(sheared, 0.3, "horizontal");
assert("shear preserves width", imageGetWidth(sheared), 30);
spun = imageNew("", 40, 40, "rgb", "red");
imageRotate(spun, 45);
assertTrue("45° rotate grows the canvas to fit", imageGetWidth(spun) GT 40);

// ---- EXIF / IPTC metadata ----------------------------------------------
// A hand-built JPEG carrying a Photoshop APP13 / IPTC-NAA segment (title,
// two keywords, caption, by-line). Values are asserted key-agnostically
// because Lucee/ACF and RustCFML name the datasets differently.
iptcJpegB64 = "/9j/7QBcUGhvdG9zaG9wIDMuMAA4QklNBAQAAAAAAEAcAgUACE15IFRpdGxlHAIZAAVhbHBoYRwCGQAEYmV0YRwCeAAOQSBjYXB0aW9uIGhlcmUcAlAACEphbmUgRG9l/+AAEEpGSUYAAQIAAAEAAQAA/8AAEQgACAAIAwERAAIRAQMRAf/bAEMACAYGBwYFCAcHBwkJCAoMFA0MCwsMGRITDxQdGh8eHRocHCAkLicgIiwjHBwoNyksMDE0NDQfJzk9ODI8LjM0Mv/bAEMBCQkJDAsMGA0NGDIhHCEyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMv/EAB8AAAEFAQEBAQEBAAAAAAAAAAABAgMEBQYHCAkKC//EALUQAAIBAwMCBAMFBQQEAAABfQECAwAEEQUSITFBBhNRYQcicRQygZGhCCNCscEVUtHwJDNicoIJChYXGBkaJSYnKCkqNDU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6g4SFhoeIiYqSk5SVlpeYmZqio6Slpqeoqaqys7S1tre4ubrCw8TFxsfIycrS09TV1tfY2drh4uPk5ebn6Onq8fLz9PX29/j5+v/EAB8BAAMBAQEBAQEBAQEAAAAAAAABAgMEBQYHCAkKC//EALURAAIBAgQEAwQHBQQEAAECdwABAgMRBAUhMQYSQVEHYXETIjKBCBRCkaGxwQkjM1LwFWJy0QoWJDThJfEXGBkaJicoKSo1Njc4OTpDREVGR0hJSlNUVVZXWFlaY2RlZmdoaWpzdHV2d3h5eoKDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uLj5OXm5+jp6vLz9PX29/j5+v/aAAwDAQACEQMRAD8A4uvmT9xP/9k=";
iptcImg = imageReadBase64(iptcJpegB64);
iptc = imageGetIPTCMetadata(iptcImg);
assertTrue("imageGetIPTCMetadata returns a struct", isStruct(iptc));
assertTrue("IPTC struct has entries", structCount(iptc) GT 0);
iptcVals = "";
for (k in iptc) { iptcVals &= iptc[k] & "|"; }
assertTrue("IPTC keyword value parsed", findNoCase("alpha", iptcVals) GT 0);
assertTrue("IPTC caption value parsed", findNoCase("A caption here", iptcVals) GT 0);
assertTrue("IPTC by-line value parsed", findNoCase("Jane Doe", iptcVals) GT 0);
// EXIF path is graceful (this fixture carries no EXIF): a struct, never a throw
assertTrue("imageGetEXIFMetadata returns a struct", isStruct(imageGetEXIFMetadata(iptcImg)));
// getBufferedImage has no engine equivalent
assertThrows("imageGetBufferedImage is unsupported", function() {
    imageGetBufferedImage(iptcImg);
});

// --- Decoding non-image bytes throws a java.io.IOException-typed error ---
// Lucee/ACF's ImageIO.read throws IOException on undecodable bytes, and CFML
// code catches that exact type (e.g. Preside's NativeImageService.resize does
// `catch("java.io.IOException"){ throw notAnImage }`). A generic runtime type
// would slip past that catch.
notImageBytes = toBinary(toBase64("this is definitely not an image"));
imgErrType = "";
try {
    imageNew(notImageBytes);
} catch (java.io.IOException e) {
    imgErrType = "io";
} catch (any e) {
    imgErrType = "other:" & e.type;
}
assert("imageNew on non-image throws java.io.IOException", imgErrType, "io");

// Regression: `<cfimage action="resize" destination="...">` died with
// "quality [true] is not a number". The image object's write() takes
// (destination, quality, overwrite), and the tag's resize/rotate/crop/convert
// branches were passing `true` POSITIONALLY as the 2nd argument — landing it in
// the QUALITY slot. It only fired when a destination was given, which is why it
// survived: the in-memory forms all worked. Found driving Preside's PDF
// thumbnail path, which resizes straight to a file.
resizeDest = tmpDir & "/rustcfml_cfimage_resize_dest.png";
if (fileExists(resizeDest)) { fileDelete(resizeDest); }
cfimage(action="resize", source=pngPath, destination=resizeDest, overwrite=true, width=20);
assertTrue("cfimage action=resize writes to a destination", fileExists(resizeDest));
assert("...at the requested width", imageInfo(imageRead(resizeDest)).width, 20);

// Regression: a builtin that writes a file must retire the cached NEGATIVE
// existence answer for that path. imageWrite/imageWriteBase64/cfimage were not
// VM-intercepted, so codegen bound them at compile time and the call skipped
// call_function — and with it every bit of the engine's filesystem bookkeeping.
// The file landed on disk and the engine went on insisting it was absent:
//
//     if ( !fileExists( thumb ) ) { imageWrite( img, thumb ); }
//     fileExists( thumb )   // false
//
// i.e. the generate-it-once idiom regenerated on every request. Being
// un-intercepted is not the same as being pure; see MUTATES_FILESYSTEM.
cacheProbe = tmpDir & "/rustcfml_img_cache_" & createUUID() & ".png";
assertFalse("a fresh path does not exist (this primes the negative cache)", fileExists(cacheProbe));
imageWrite(imageNew("", 10, 10), cacheProbe);
assertTrue("imageWrite retires the cached negative for the path it wrote", fileExists(cacheProbe));
fileDelete(cacheProbe);

cfimageProbe = tmpDir & "/rustcfml_img_cache2_" & createUUID() & ".png";
assertFalse("likewise for cfimage's destination", fileExists(cfimageProbe));
cfimage(action="resize", source=pngPath, destination=cfimageProbe, width=12, overwrite=true);
assertTrue("cfimage action=resize retires it too", fileExists(cfimageProbe));
fileDelete(cfimageProbe);

rotateDest = tmpDir & "/rustcfml_cfimage_rotate_dest.png";
if (fileExists(rotateDest)) { fileDelete(rotateDest); }
cfimage(action="rotate", source=pngPath, destination=rotateDest, overwrite=true, angle=90);
assertTrue("cfimage action=rotate writes to a destination too", fileExists(rotateDest));

// cleanup
for (p in [pngPath, jpgPath, misnamed, resizeDest, rotateDest, tmpDir & "/rustcfml_cfimage_write.png"]) {
    if (fileExists(p)) { fileDelete(p); }
}

suiteEnd();
</cfscript>
