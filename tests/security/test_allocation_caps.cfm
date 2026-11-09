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
<cfscript>
suiteBegin("Password-hash verification refuses absurd cost parameters from the hash");
// A stored hash is often attacker-supplied; its cost must not decide how long
// (or how much memory) one request spends.
assertThrows("bcrypt cost 31 in the hash", function() {
	verifyBCryptHash("pw", "$2a$31$abcdefghijklmnopqrstuuJmQbIbkVpvdmuKWGhCRJOvEfiNM4xWW");
});
assertThrows("BCryptHash cost above the ceiling", function() { generateBCryptHash("pw", 25); });
assertThrows("scrypt ln=30", function() {
	verifySCryptHash("pw", "$scrypt$ln=30,r=8,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYXNoaGFzaGhhc2g");
});
assertThrows("argon2 m=4194304", function() {
	argon2CheckHash("$argon2id$v=19$m=4194304,t=3,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYXNoaGFzaGhhc2g", "pw");
});
h = generateBCryptHash("pw", 4);
assertTrue("an ordinary bcrypt round-trip still works", verifyBCryptHash("pw", h));
suiteEnd();
</cfscript>
