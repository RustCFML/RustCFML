<cfscript>
suiteBegin("Unknown hash algorithms throw (no silent MD5)");

// Both `hash()` and `MessageDigest.getInstance()` used to fall back to MD5 for
// any algorithm they did not implement. A caller asking for SHA-512 — or making
// a typo — got a plausible-looking MD5 digest and no indication anything was
// wrong. Lucee 7.0.4 throws java.security.NoSuchAlgorithmException from both;
// the messages below are byte-for-byte what Lucee reports.

assert("hash() still hashes the algorithms it supports",
	hash("abc", "MD5"), "900150983CD24FB0D6963F7D28E17F72");
assert("hash() supports SHA-256",
	hash("abc", "SHA-256"),
	"BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD");

// Unknown algorithm: must throw, not downgrade.
caught = "";
try {
	hash("abc", "BOGUS-ALG");
} catch (any e) {
	caught = e.type & " :: " & e.message;
}
assert("hash() throws NoSuchAlgorithmException for an unknown algorithm",
	caught, "java.security.NoSuchAlgorithmException :: bogus-alg MessageDigest not available");

caught = "";
try {
	createObject("java", "java.security.MessageDigest").getInstance("BOGUS-ALG");
} catch (any e) {
	caught = e.type & " :: " & e.message;
}
assert("MessageDigest.getInstance throws for an unknown algorithm",
	caught, "java.security.NoSuchAlgorithmException :: BOGUS-ALG MessageDigest not available");

// The supported set must keep working through the shim, including Java's
// documented "SHA" alias for SHA-1.
md = createObject("java", "java.security.MessageDigest").getInstance("SHA-256");
// `.getBytes()` matters: real Java MessageDigest.update takes a byte[] and
// Lucee rejects a bare string ("No matching method for ...update(string)").
// Our shim is deliberately lenient and accepts both, so passing a string here
// would make this suite silently RustCFML-only.
md.update("abc".getBytes());
assert("MessageDigest SHA-256 still digests correctly",
	ucase(binaryEncode(md.digest(), "hex")),
	"BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD");

shaAlias = createObject("java", "java.security.MessageDigest").getInstance("SHA");
shaAlias.update("abc".getBytes());
assert("MessageDigest accepts SHA as an alias for SHA-1",
	ucase(binaryEncode(shaAlias.digest(), "hex")),
	"A9993E364706816ABA3E25717850C26C9CD0D89D");

suiteEnd();
</cfscript>
