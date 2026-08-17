<cfscript>
// java.security.Signature / KeyFactory — the asymmetric-crypto shim surface that
// vendored JWT libraries (jwt-cfml et al.) sign and verify through. This is the
// exact call shape of titan's code/shared/lib/jwt.cfc + encodingUtils.cfc
// (vendored jwt-cfml): the ONE remaining gap between titan and full parity on
// RustCFML — Auth0 RS256 id-token verification currently runs claims-only,
// gated on canVerifyAsymmetric(), i.e. on createObject("java",
// "java.security.Signature") not throwing.
//
// On stock RustCFML every leg dies at construction:
//   createObject: Java class [java.security.Signature] is not supported.
// (java.security stops at MessageDigest — docs/java-shims.md.)
//
// Scope is deliberately minimal — verify/sign only, RSA only, the tier-1 "run
// vendored libs unmodified" surface:
//   - Signature: getInstance("SHA256withRSA"), initVerify, initSign, update
//     (incl. chunked), verify -> boolean (false, not throw, on mismatch), sign
//   - KeyFactory: getInstance("RSA"), generatePublic, generatePrivate
//   - Key specs: X509EncodedKeySpec (PEM public), PKCS8EncodedKeySpec (PEM
//     private), RSAPublicKeySpec (JWKS n/e path)
//   - BigInteger: init(signum, magnitudeBytes) + bitLength() — the JWKS
//     modulus constructor; signum=1 with a high bit set is the case a
//     two's-complement misread gets wrong
// NOT pinned here: EC/ECDSA, DER<->P1363, CertificateFactory, KeyPairGenerator
// — jwt-cfml only reaches those for ES* algorithms and X.509-wrapped keys.
//
// All expected values are measured: the keypair/signature fixtures were
// generated with OpenSSL and cross-checked on Lucee. SHA256withRSA is RSASSA-
// PKCS1-v1_5 — DETERMINISTIC — so sign() is asserted byte-for-byte:
//   openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048
//   openssl dgst -sha256 -sign priv.pem signing_input.txt
// The JWT fixture is an Auth0-shaped RS256 id token (iss/sub/aud/iat/exp, kid
// in the header) — the real-world verify target.

suiteBegin("java.security Signature/KeyFactory: RS256 verify + sign (vendored jwt-cfml surface)");

function b64UrlToBin(required string s) {
	var t = replace(replace(arguments.s, "-", "+", "all"), "_", "/", "all");
	var pad = len(t) % 4;
	if (pad == 2) { t = t & "=="; }
	else if (pad == 3) { t = t & "="; }
	return binaryDecode(t, "base64");
}

// ── Fixtures (measured; see header comment for provenance) ──

// X509 SubjectPublicKeyInfo DER, base64 (PEM body of `openssl pkey -pubout`)
PUB_B64 = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAy81oUAXPoOfokSx4VmLnM8KGiZ5n74L0uupXsFF09t3EooEA1XvhY3z1do5wkFF41sVKmim2IcBtNz+V6cEa46ShQkwguUEVK3vsWpkp+7krWIWH5UuD6sK/YkjvMIv9KPRJTIiIY9lTQc9nChD6oY86STbKeKwIFSqGFpZ96lLDoVeF5gRvuH83WPOr0rO/i5NXH90N1Vzfl5zrn3w+AkqtHi3hbn1wRLNhV06UxiXdixIR008sDwB0iRp/ivtXeDuonk1jH/KbWZvWl9zz22aX5/jGQs3K/XW4Q7qjXwNRUHPiRe++jeYTmqoH9vtvvYOZo71QsOJ3aXUGHDwI+QIDAQAB";

// PKCS8 PrivateKeyInfo DER, base64 (PEM body of `openssl genpkey`)
PRIV_B64 = "MIIEuwIBADANBgkqhkiG9w0BAQEFAASCBKUwggShAgEAAoIBAQDLzWhQBc+g5+iR"
	& "LHhWYuczwoaJnmfvgvS66lewUXT23cSigQDVe+FjfPV2jnCQUXjWxUqaKbYhwG03"
	& "P5XpwRrjpKFCTCC5QRUre+xamSn7uStYhYflS4Pqwr9iSO8wi/0o9ElMiIhj2VNB"
	& "z2cKEPqhjzpJNsp4rAgVKoYWln3qUsOhV4XmBG+4fzdY86vSs7+Lk1cf3Q3VXN+X"
	& "nOuffD4CSq0eLeFufXBEs2FXTpTGJd2LEhHTTywPAHSJGn+K+1d4O6ieTWMf8ptZ"
	& "m9aX3PPbZpfn+MZCzcr9dbhDuqNfA1FQc+JF776N5hOaqgf2+2+9g5mjvVCw4ndp"
	& "dQYcPAj5AgMBAAECggEAEKJ9NWzn/9j1/2V7FAgCGZQy4Yg7sQ3GFnyauDJJ4v2C"
	& "d4YikIpKQRHZwjnJn8n6nEMhvfgSHOqlL3mB7cC8hmCxZeUrVZulk/VLOzDdv6Vj"
	& "T5gkmbdidta7AtVzqol+miloUzYgtc+vD0PTw/tTTb0QU5oEiDl4dmvQYocZk3bU"
	& "7IgamRHkQO2hXhg59PRLDBZtShh2M+Vr5tu6Nkh5DQMZx6Q9FQ4DaprIMvbsOsOH"
	& "WqXERI+7XyXD9HrIQRan6dL0NYn+DQBxiBXTPyKcW7xkPhX0guY3+PI3hiwze10e"
	& "GvZ3FIAjahZvS2a7Hgtp8+OJTBJWFdu5LG0OvTLlNQKBgQD+1c2/QmTz3pWwCS7i"
	& "mGDcTJ1J0QluWrwOSgWmVY+pXJe1Ev5CM0Ez4IhVBQ3fmj1AVl9xBOuNewgBJPpt"
	& "YG7iH5ch2DnScLSF6uEPswF+o260QxadH6Y2bUmMmOW+bp4wT2KhXmo6sguaoigM"
	& "S8VkesJ0cClHr2IpGrkeEfKgZQKBgQDMu+M2/M4qOjr3ojK8Rhi9u0uaQhi59noD"
	& "3N59oMzMqjC/XoQ196sZMtqS3zXSp5I56u6fBRDjubYXhH1C+YkGSF2ZvMthWqSt"
	& "Cu4o3xjoJJwvF40+1ebxgYHZKrUJttR+ejPXGxPDPB0GCaCfbEDlTrgjpyWfBE9b"
	& "wp7+Jl9bBQKBgQDh+62h7vnhPUDWw739GY3DrnlJDYNkhjgAH+pUr8lfMgoifD5X"
	& "bGZk37dmVb4QzRGGLVIjwm40n6bghO9C8WJDSipWzA2yrVmY7Eo7Bs3LhJjWaCv0"
	& "mC1oVJAFi00pC6ViR/O6ECT5+gvKTARhqwvX5/jrEs+4jzHoK5d/sADN9QJ/aMO8"
	& "gWbcGL+zbQSS8cTs5CnzhfxMqtx1k4RyVdby9cghKcwz22nhJAPh1UZjRGh7ayfM"
	& "85KkEeP2ShKCBsOqWoytWP5DFI5Ntj7hoAiJtkEFqxNiM8VBaFPdHkO8YDwugIoH"
	& "/QreEgmw0GYcy3eZvb2KfLQLjFOoSExPD76TDQKBgBNWtDVWsrxKwGwUiDQYTpGq"
	& "qRnJFxXqqLhXFt5KQMaYT5WQ+qU0tVxGu2La2kcAUHldrPw2lP4FO9+eO7bY2ROy"
	& "0wbsak9dzMJOLJO7nlvIjY3ADfRAURpn4/O5oRonESYDZK5vsh1PJzBRFDxlmtiA"
	& "N529+RccgdKtYBuk/lbq";

// JWT signing input: base64url(header).base64url(payload) — Auth0-shaped
// RS256 id token, kid in the header, iss/sub/aud/email/iat/exp claims.
SIGNING_INPUT = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6InRpdGFuLXRlc3QtMSJ9"
	& ".eyJpc3MiOiJodHRwczovL3RpdGFuLXRlc3QuYXUuYXV0aDAuY29tLyIsInN1YiI6ImF1dGgwfDY0ZjBjMSIsImF1ZCI6Imh1Yi50aXRhbi50ZXN0IiwiZW1haWwiOiJqYW5lQGV4YW1wbGUudGVzdCIsImlhdCI6MTc1NTA0MzIwMCwiZXhwIjozMjUwMzY4MDAwMH0";

// SHA256withRSA over SIGNING_INPUT with PRIV — deterministic (PKCS#1 v1.5)
SIG_B64 = "KaC4l29yVgAnaSM/eYOzfWpPtLqAlzVCnfuVRTOqixlYNXVWP+y8wIKIMojjALfM9wzEeTqSN3Jv4wL/TAMGVw3+sdmQJC0Tma8Fhhom6VuUKi+LrCws/eQWLjEGHhe+jcNOBb2RAesMxlQUzGOAG4qTmltLt2Gl/4hdpFKjrO66CUeXnSQr6Mhf3XmDor7UnA/2Otz+psusuWANUd4Dm9mtmBOgs+uUrfOyLSFItXIGpyuuDBlZUGIYpq2mk53+MMQstcokW7UC3ygPj9AFET2uNXiTLfTZPHS2hIdSVLofwuEGRMlcErKMDqKuDYtRkSDATm+w3tn0jJh3EXTYzQ==";

// The same key as a JWKS entry: base64url modulus + exponent (RFC 7518 §6.3)
JWK_N = "y81oUAXPoOfokSx4VmLnM8KGiZ5n74L0uupXsFF09t3EooEA1XvhY3z1do5wkFF41sVKmim2IcBtNz-V6cEa46ShQkwguUEVK3vsWpkp-7krWIWH5UuD6sK_YkjvMIv9KPRJTIiIY9lTQc9nChD6oY86STbKeKwIFSqGFpZ96lLDoVeF5gRvuH83WPOr0rO_i5NXH90N1Vzfl5zrn3w-AkqtHi3hbn1wRLNhV06UxiXdixIR008sDwB0iRp_ivtXeDuonk1jH_KbWZvWl9zz22aX5_jGQs3K_XW4Q7qjXwNRUHPiRe--jeYTmqoH9vtvvYOZo71QsOJ3aXUGHDwI-Q";
JWK_E = "AQAB";

// The complete token (signing input + base64url signature), as Auth0 sends it
ID_TOKEN = SIGNING_INPUT & ".KaC4l29yVgAnaSM_eYOzfWpPtLqAlzVCnfuVRTOqixlYNXVWP-y8wIKIMojjALfM9wzEeTqSN3Jv4wL_TAMGVw3-sdmQJC0Tma8Fhhom6VuUKi-LrCws_eQWLjEGHhe-jcNOBb2RAesMxlQUzGOAG4qTmltLt2Gl_4hdpFKjrO66CUeXnSQr6Mhf3XmDor7UnA_2Otz-psusuWANUd4Dm9mtmBOgs-uUrfOyLSFItXIGpyuuDBlZUGIYpq2mk53-MMQstcokW7UC3ygPj9AFET2uNXiTLfTZPHS2hIdSVLofwuEGRMlcErKMDqKuDYtRkSDATm-w3tn0jJh3EXTYzQ";

sigBytes = binaryDecode(SIG_B64, "base64");

// ── A: every class the vendored lib constructs must be constructible ──
// jwt-cfml's init() does createObject("java","java.security.Signature") once
// and gates all asymmetric work on it (titan: canVerifyAsymmetric()).
for (cls in [
	"java.security.Signature",
	"java.security.KeyFactory",
	"java.security.spec.X509EncodedKeySpec",
	"java.security.spec.PKCS8EncodedKeySpec",
	"java.security.spec.RSAPublicKeySpec",
	"java.math.BigInteger"
]) {
	clsResult = "ok";
	try { createObject("java", cls); }
	catch (any e) { clsResult = "THREW: " & e.message; }
	assert("A createObject " & cls, clsResult, "ok");
}

// ── B: PEM public key (X509EncodedKeySpec) verifies the RS256 signature ──
// encodingUtils.parsePEMEncodedKey(): X509EncodedKeySpec -> KeyFactory("RSA")
// .generatePublic(); jwt.cfc verifySignature(): getInstance/initVerify/update/
// verify. getInstance is a STATIC call on the un-init'ed class object — the
// exact shape jwt.cfc uses (variables.jss.getInstance(...)).
verifyB = "THREW";
algB = "(missing)";
try {
	pubSpec = createObject("java", "java.security.spec.X509EncodedKeySpec").init(binaryDecode(PUB_B64, "base64"));
	pubKey = createObject("java", "java.security.KeyFactory").getInstance("RSA").generatePublic(pubSpec);
	algB = pubKey.getAlgorithm();
	jss = createObject("java", "java.security.Signature");
	instB = jss.getInstance("SHA256withRSA");
	instB.initVerify(pubKey);
	instB.update(charsetDecode(SIGNING_INPUT, "utf-8"));
	verifyB = instB.verify(sigBytes);
} catch (any e) { verifyB = "THREW: " & e.message; }
assert("B1 X509 PEM public key verifies the measured RS256 signature", verifyB, true);
assert("B2 generatePublic returns an RSA key (getAlgorithm)", algB, "RSA");

// ── C: a wrong signature makes verify() return FALSE — it must not throw ──
// jwt.cfc turns false into jwtcfml.InvalidSignature; a throw here would
// surface as an engine error instead of the library's typed error.
verifyC = "THREW";
try {
	instC = jss.getInstance("SHA256withRSA");
	instC.initVerify(pubKey);
	instC.update(charsetDecode(SIGNING_INPUT & "x", "utf-8"));
	verifyC = instC.verify(sigBytes);
} catch (any e) { verifyC = "THREW: " & e.message; }
assert("C1 tampered payload: verify() returns false, does not throw", verifyC, false);

// ── D: JWKS path — BigInteger(signum, bytes) + RSAPublicKeySpec ──
// encodingUtils.parseJWK(): BigInteger.init(1, base64UrlToBinary(jwk.n)) —
// signum-magnitude, NOT two's-complement. This modulus has its high bit set:
// a two's-complement misread yields a negative BigInteger / wrong bitLength.
bitLenD = "THREW";
verifyD = "THREW";
try {
	bigN = createObject("java", "java.math.BigInteger").init(1, b64UrlToBin(JWK_N));
	bigE = createObject("java", "java.math.BigInteger").init(1, b64UrlToBin(JWK_E));
	bitLenD = bigN.bitLength();
	jwkSpec = createObject("java", "java.security.spec.RSAPublicKeySpec").init(bigN, bigE);
	jwkKey = createObject("java", "java.security.KeyFactory").getInstance("RSA").generatePublic(jwkSpec);
	instD = jss.getInstance("SHA256withRSA");
	instD.initVerify(jwkKey);
	instD.update(charsetDecode(SIGNING_INPUT, "utf-8"));
	verifyD = instD.verify(sigBytes);
} catch (any e) { verifyD = "THREW: " & e.message; }
assert("D1 BigInteger(1, jwkN) is signum-magnitude (bitLength 2048)", bitLenD, 2048);
assert("D2 JWKS n/e (RSAPublicKeySpec) key verifies the same signature", verifyD, true);

// ── E: sign with the PKCS8 private key — byte-identical (deterministic) ──
// SHA256withRSA is RSASSA-PKCS1-v1_5: same key + same input -> same bytes,
// so the OpenSSL-measured signature is the exact expected output.
signE = "THREW";
algE = "(missing)";
try {
	privSpec = createObject("java", "java.security.spec.PKCS8EncodedKeySpec").init(binaryDecode(PRIV_B64, "base64"));
	privKey = createObject("java", "java.security.KeyFactory").getInstance("RSA").generatePrivate(privSpec);
	algE = privKey.getAlgorithm();
	instE = jss.getInstance("SHA256withRSA");
	instE.initSign(privKey);
	instE.update(charsetDecode(SIGNING_INPUT, "utf-8"));
	signE = binaryEncode(instE.sign(), "base64");
} catch (any e) { signE = "THREW: " & e.message; }
assert("E1 sign() output is byte-identical to the OpenSSL signature", signE, SIG_B64);
assert("E2 generatePrivate returns an RSA key (getAlgorithm)", algE, "RSA");

// ── F: the full id-token flow, as auth0/callback.cfc runs it ──
// Split the token, rebuild the signing input from parts 1+2, base64url-decode
// part 3, verify with the JWKS-derived key.
verifyF = "THREW";
try {
	parts = listToArray(ID_TOKEN, ".");
	instF = jss.getInstance("SHA256withRSA");
	instF.initVerify(jwkKey);
	instF.update(charsetDecode(parts[1] & "." & parts[2], "utf-8"));
	verifyF = instF.verify(b64UrlToBin(parts[3]));
} catch (any e) { verifyF = "THREW: " & e.message; }
assert("F1 full JWT: rebuilt signing input verifies against part 3", verifyF, true);

// ── G: an unknown algorithm raises NoSuchAlgorithmException ──
// Same contract the MessageDigest shim already honours (docs/java-shims.md):
// fail loudly, never silently substitute.
nsaeG = "no throw";
try {
	jssG = createObject("java", "java.security.Signature");
	jssG.getInstance("SHA256withNOPE");
} catch (any e) { nsaeG = e.type & " " & e.message; }
assertTrue("G1 getInstance(unknown alg) raises NoSuchAlgorithmException, got: [" & nsaeG & "]",
	findNoCase("NoSuchAlgorithm", nsaeG) > 0);

// ── H: update() accumulates across chunked calls ──
// Streaming shape: two update() calls over the split input must verify the
// same as one call over the whole (ASCII input, so char split == byte split).
verifyH = "THREW";
try {
	half = int(len(SIGNING_INPUT) / 2);
	instH = jss.getInstance("SHA256withRSA");
	instH.initVerify(pubKey);
	instH.update(charsetDecode(left(SIGNING_INPUT, half), "utf-8"));
	instH.update(charsetDecode(right(SIGNING_INPUT, len(SIGNING_INPUT) - half), "utf-8"));
	verifyH = instH.verify(sigBytes);
} catch (any e) { verifyH = "THREW: " & e.message; }
assert("H1 chunked update() accumulates before verify", verifyH, true);

suiteEnd();
</cfscript>
