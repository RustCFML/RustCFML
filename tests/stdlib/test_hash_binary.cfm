<cfscript>
suiteBegin("hash() digests binary BYTES, not a string form of them (GH 376)");

// hash() read its input through a lossy string coercion, so a CFML binary was
// digested as mojibake rather than as its bytes. The result was a
// plausible-looking but WRONG digest — nothing throws, nothing looks broken —
// which silently breaks AWS SigV4 payload signing and every other protocol that
// hashes bytes. These are the reference digests every other engine produces.

abcBytes = charsetDecode("abc", "utf-8");

assert("SHA-256 of the three bytes 'abc'",
	hash(abcBytes, "SHA-256"),
	"BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD");

assert("hashing the bytes agrees with hashing the equivalent string",
	hash(abcBytes, "SHA-256"), hash("abc", "SHA-256"));

assert("MD5 of the three bytes 'abc'",
	hash(abcBytes, "MD5"), "900150983CD24FB0D6963F7D28E17F72");

assert("SHA-1 of the three bytes 'abc'",
	hash(abcBytes, "SHA-1"), "A9993E364706816ABA3E25717850C26C9CD0D89D");

// The empty-payload digest AWS SigV4 uses for a body-less request. It is the
// single most load-bearing binary hash in practice, and the old coercion got it
// wrong too.
assert("SHA-256 of an empty byte array",
	hash(binaryDecode("", "hex"), "SHA-256"),
	"E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855");

// Bytes that are not valid UTF-8 have no string form at all, so a coercion-based
// implementation cannot even round-trip them.
assert("SHA-256 of non-UTF-8 bytes (0xFF 0xFE 0x00 0x01)",
	hash(binaryDecode("FFFE0001", "hex"), "SHA-256"),
	"D2AD9277BAAEE14856D20EC2B21F87A0CB8A7F86C6EF090FD5A082B1E85135AC");

suiteEnd();
</cfscript>
