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

// cleanup
for (p in [pngPath, jpgPath, misnamed, tmpDir & "/rustcfml_cfimage_write.png"]) {
    if (fileExists(p)) { fileDelete(p); }
}

suiteEnd();
</cfscript>
