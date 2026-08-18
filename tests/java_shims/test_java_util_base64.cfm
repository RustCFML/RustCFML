<cfscript>
// java.util.Base64 — the JDK's standard base64 API, unshimmed:
//
//   createObject: Java class [java.util.Base64] is not supported.
//
// Repro class: JWK-to-PEM builders (the JWKS verification path the v0.606.0
// Signature/KeyFactory shims enabled) conventionally finish with
// Base64.getEncoder().encodeToString(publicKey.getEncoded()) — titan's Auth0
// callback did exactly that and had to switch to binaryEncode(). The pure-CFML
// spelling exists, but any Lucee codebase reaching this class dies at
// createObject before it can find out.

suiteBegin("java.util.Base64: encoder/decoder shim (JWK-to-PEM surface)");

b64 = "(threw)";
try {
    b64 = createObject("java", "java.util.Base64");
} catch (any e) {
    b64 = "THREW: " & e.message;
}

if ( isSimpleValue(b64) ) {
    assert( "java.util.Base64 resolves via createObject", b64, "(an object)" );
} else {
    assertTrue( "java.util.Base64 resolves via createObject", true );

    enc = "(threw)";
    try { enc = b64.getEncoder().encodeToString(charsetDecode("AB", "utf-8")); }
    catch (any e) { enc = "THREW: " & e.message; }
    assert( "getEncoder().encodeToString() matches binaryEncode", enc, "QUI=" );

    dec = "(threw)";
    try { dec = charsetEncode(b64.getDecoder().decode("QUI="), "utf-8"); }
    catch (any e) { dec = "THREW: " & e.message; }
    assert( "getDecoder().decode() round-trips", dec, "AB" );
}

suiteEnd();
</cfscript>
