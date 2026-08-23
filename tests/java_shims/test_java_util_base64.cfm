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

    // Added with the shim: the rest of the surface, each value measured on
    // Lucee 7.1.0.204. The URL-safe legs are the point of the class for JWKS
    // work — base64url is what a JWT carries — and the padding/wrapping legs
    // are where a hand-rolled encoder usually diverges.
    assert( "getEncoder() pads a partial group", b64.getEncoder().encodeToString(charsetDecode("A", "utf-8")), "QQ==" );
    assert( "withoutPadding() drops the padding", b64.getEncoder().withoutPadding().encodeToString(charsetDecode("A", "utf-8")), "QQ" );
    assert( "getUrlEncoder() uses the URL-safe alphabet", b64.getUrlEncoder().encodeToString(binaryDecode("fbff", "hex")), "-_8=" );
    assert( "getEncoder() uses the standard alphabet for the same bytes", b64.getEncoder().encodeToString(binaryDecode("fbff", "hex")), "+/8=" );
    assert( "getUrlDecoder() reads the URL-safe alphabet", binaryEncode(b64.getUrlDecoder().decode("--8="), "hex"), "FBEF" );
    assert( "encode() returns bytes, not a string", charsetEncode(b64.getEncoder().encode(charsetDecode("AB", "utf-8")), "utf-8"), "QUI=" );
    assert( "decode() also accepts a byte[]", charsetEncode(b64.getDecoder().decode(charsetDecode("QUI=", "utf-8")), "utf-8"), "AB" );

    // MIME wraps at 76 characters with CRLF; the basic encoder never wraps.
    longIn = repeatString("A", 200);
    assert( "getMimeEncoder() wraps at 76 chars with CRLF",
        listLen( b64.getMimeEncoder().encodeToString(charsetDecode(longIn, "utf-8")), chr(13) & chr(10) ), 4 );
    assert( "getEncoder() never wraps",
        len( b64.getEncoder().encodeToString(charsetDecode(longIn, "utf-8")) ), 268 );
    assertTrue( "getMimeDecoder() tolerates the line breaks it produced",
        charsetEncode( b64.getMimeDecoder().decode( b64.getMimeEncoder().encodeToString(charsetDecode(longIn, "utf-8")) ), "utf-8" ) == longIn );
}

suiteEnd();
</cfscript>
