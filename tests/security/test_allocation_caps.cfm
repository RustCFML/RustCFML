<cfscript>
suiteBegin("One call cannot ask for an allocation that aborts the process");

// A failed allocation is not an exception in Rust: it aborts the whole
// process, taking every in-flight request with it. These used to request
// terabytes in a single allocation.
assertThrows("repeatString with a huge count", function() {
	repeatString("a", 4000000000000);
});
assertThrows("repeatString whose count * length overflows the cap", function() {
	repeatString(repeatString("x", 1000), 1000000);
});
assert("repeatString of an ordinary size still works", len(repeatString("ab", 1000)), 2000);

assertThrows("StringBuilder.setLength to a huge length", function() {
	sb = createObject("java", "java.lang.StringBuilder").init("x");
	sb.setLength(4000000000000);
});

assertThrows("imageNew with a huge canvas", function() {
	imageNew("", 46000, 46000);
});
assertThrows("imageResize to a huge size", function() {
	img = imageNew("", 10, 10);
	imageResize(img, 46000, 46000);
});
assertThrows("imageScaleToFit that would upscale past the budget", function() {
	img = imageNew("", 1, 1);
	imageScaleToFit(img, 100000, 100000);
});
small = imageNew("", 64, 32);
imageResize(small, 128, 64);
assert("an ordinary resize still works", imageGetWidth(small), 128);

suiteEnd();
</cfscript>
