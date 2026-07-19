<cfscript>
// GH #276 — shim java.nio.ByteBuffer + java.io.ByteArrayOutputStream, the two
// remaining plain java.* standard-library classes Preside's GoogleAuthenticator
// (TOTP/2FA base32 encode/decode) leans on. Semantics cross-checked against the
// Lucee 5.x oracle recorded in the issue.
suiteBegin("Java ByteBuffer + ByteArrayOutputStream shims (GH-276)");

// ---- java.nio.ByteBuffer ----

// allocate(n) reserves a zero-filled backing array; array() returns ALL of it.
buf = createObject("java", "java.nio.ByteBuffer").allocate(8);
buf.putLong(javaCast("long", 1));
assert("allocate(8).array() length is capacity", arrayLen(buf.array()), 8);

// putLong writes 8 bytes BIG-ENDIAN: counter 1 -> [0,0,0,0,0,0,0,1].
la = buf.array();
assert("putLong(1) first byte", la[1], 0);
assert("putLong(1) last byte", la[8], 1);

// put(byte[], offset, length) writes a slice; the rest stays zero-padded — the
// exact behaviour base32Encode's padding branch relies on.
inputBytes = javaCast("string", "abcdef").getBytes(); // 6 bytes: a..f
pbuf = createObject("java", "java.nio.ByteBuffer").allocate(10);
pbuf.put(inputBytes, 0, arrayLen(inputBytes));
pa = pbuf.array();
assert("put(...) keeps full capacity", arrayLen(pa), 10);
assert("put(...) first written byte", pa[1], asc("a")); // 97
assert("put(...) last written byte", pa[6], asc("f"));  // 102
assert("put(...) zero pad [7]", pa[7], 0);
assert("put(...) zero pad [10]", pa[10], 0);

// A buffer mutated inside a function is visible to the caller (shared handle).
function fillIt(required any b) { arguments.b.putInt(javaCast("int", 258)); } // 0x00000102
sbuf = createObject("java", "java.nio.ByteBuffer").allocate(4);
fillIt(sbuf);
sa = sbuf.array();
assert("putInt via function-arg mutates caller buffer [3]", sa[3], 1);
assert("putInt via function-arg mutates caller buffer [4]", sa[4], 2);

// capacity()/remaining()/position() cursor bookkeeping.
cbuf = createObject("java", "java.nio.ByteBuffer").allocate(6);
cbuf.put(javaCast("int", 65));
assert("capacity()", cbuf.capacity(), 6);
assert("position() after 1 write", cbuf.position(), 1);
assert("remaining()", cbuf.remaining(), 5);

// wrap(byte[]) exposes an existing array; get() reads it signed & advances.
wbuf = createObject("java", "java.nio.ByteBuffer").wrap(javaCast("string", "Hi").getBytes());
assert("wrap().capacity()", wbuf.capacity(), 2);
assert("wrap().get() reads first byte", wbuf.get(), asc("H")); // 72

// ---- java.io.ByteArrayOutputStream ----

baos = createObject("java", "java.io.ByteArrayOutputStream").init();
baos.write(72);  // 'H'
baos.write(105); // 'i'
assert("BAOS toByteArray() length", arrayLen(baos.toByteArray()), 2);
assert("BAOS toByteArray()[1]", baos.toByteArray()[1], 72);
assert("BAOS size()", baos.size(), 2);
assert("BAOS toString()", baos.toString(), "Hi");

// write(int) keeps only the low 8 bits (Java's OutputStream.write(int) contract).
lowbits = createObject("java", "java.io.ByteArrayOutputStream").init();
lowbits.write(256 + 65); // 321 -> 65 ('A')
assert("BAOS write() masks to low 8 bits", lowbits.toString(), "A");

// reset() empties it.
baos.reset();
assert("BAOS reset() clears", baos.size(), 0);

// charsetEncode() accepts the byte[]-as-Array that toByteArray()/getBytes()
// return (base32decodeString does charsetEncode(base32decode(x), enc)).
enc = createObject("java", "java.io.ByteArrayOutputStream").init();
enc.write(72); enc.write(105);
assert("charsetEncode(toByteArray())", charsetEncode(enc.toByteArray(), "utf-8"), "Hi");

// ---- Preside base32 encode/decode round-trip (verbatim algorithm) ----
// This is the exact code path #276 unblocks; it exercises getBytes + ByteBuffer
// (encode padding) + ByteArrayOutputStream (decode) together.

DECODE_TABLE = [
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
      -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,63,
      -1,-1,26,27,28,29,30,31,-1,-1,-1,-1,-1,-1,-1,-1,
      -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,
      15,16,17,18,19,20,21,22,23,24,25
];

function base32encode(required any inputBytes) {
    var values = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    if (arrayLen(inputBytes) == 0) return "";
    var bytes = 0;
    if (arrayLen(inputBytes) % 5 != 0) {
        var paddedLength = arrayLen(inputBytes) + (5 - (arrayLen(inputBytes) % 5));
        var buffer = createObject("java", "java.nio.ByteBuffer").allocate(paddedLength);
        buffer.put(inputBytes, 0, arrayLen(inputBytes));
        bytes = buffer.array();
    } else {
        bytes = inputBytes;
    }
    var encoded = "";
    var byte = 0; var byte2 = 0;
    for (var i = 1; i <= arrayLen(bytes); i += 5) {
        byte = bytes[i]; if (byte < 0) byte += 256; byte = bitAnd(bitSHRN(byte,3),31); encoded &= mid(values, byte+1, 1);
        byte = bytes[i]; if (byte < 0) byte += 256; byte = bitSHLN(bitAnd(byte,7),2);
        byte2 = bytes[i+1]; if (byte2 < 0) byte2 += 256; byte2 = bitAnd(bitSHRN(byte2,6),3);
        byte = bitOr(byte, byte2); encoded &= mid(values, byte+1, 1);
        byte = bytes[i+1]; if (byte < 0) byte += 256; byte = bitSHRN(bitAnd(byte,62),1); encoded &= mid(values, byte+1, 1);
        byte = bytes[i+1]; if (byte < 0) byte += 256; byte = bitSHLN(bitAnd(byte,1),4);
        byte2 = bytes[i+2]; if (byte2 < 0) byte2 += 256; byte2 = bitSHRN(byte2,4);
        byte = bitOr(byte, byte2); encoded &= mid(values, byte+1, 1);
        byte = bytes[i+2]; if (byte < 0) byte += 256; byte = bitSHLN(bitAnd(byte,15),1);
        byte2 = bytes[i+3]; if (byte2 < 0) byte2 += 256; byte2 = bitSHRN(byte2,7);
        byte = bitOr(byte, byte2); encoded &= mid(values, byte+1, 1);
        byte = bytes[i+3]; if (byte < 0) byte += 256; byte = bitAnd(bitSHRN(byte,2),31); encoded &= mid(values, byte+1, 1);
        byte = bytes[i+3]; if (byte < 0) byte += 256; byte = bitSHLN(bitAnd(byte,3),3);
        byte2 = bytes[i+4]; if (byte2 < 0) byte2 += 256; byte2 = bitSHRN(byte2,5);
        byte = bitOr(byte, byte2); encoded &= mid(values, byte+1, 1);
        byte = bytes[i+4]; if (byte < 0) byte += 256; byte = bitAnd(byte,31); encoded &= mid(values, byte+1, 1);
    }
    encoded = left(encoded, (arrayLen(inputBytes)/5)*8 + 1);
    if (len(encoded) % 8 != 0) encoded &= repeatString("=", 8 - (len(encoded) % 8));
    return encoded;
}

function base32decode(required string encoded) {
    var byte = 0; var byte2 = 0; var byte3 = 0;
    var encodedBytes = javaCast("string", encoded).getBytes();
    var decodedBytes = createObject("java", "java.io.ByteArrayOutputStream").init();
    for (var i = 1; i <= arrayLen(encodedBytes); i += 8) {
        if (encodedBytes[i+1] == 61) break;
        byte  = bitSHLN(DECODE_TABLE[encodedBytes[i]], 3);
        byte2 = bitSHRN(DECODE_TABLE[encodedBytes[i+1]], 2);
        decodedBytes.write(bitOr(byte, byte2));
        if (encodedBytes[i+3] == 61) break;
        byte  = bitSHLN(bitAnd(DECODE_TABLE[encodedBytes[i+1]], 3), 6);
        byte2 = bitSHLN(DECODE_TABLE[encodedBytes[i+2]], 1);
        byte3 = bitSHRN(DECODE_TABLE[encodedBytes[i+3]], 4);
        decodedBytes.write(bitOr(bitOr(byte, byte2), byte3));
        if (encodedBytes[i+4] == 61) break;
        byte  = bitSHLN(bitAnd(DECODE_TABLE[encodedBytes[i+3]], 15), 4);
        byte2 = bitSHRN(DECODE_TABLE[encodedBytes[i+4]], 1);
        decodedBytes.write(bitOr(byte, byte2));
        if (encodedBytes[i+5] == 61) break;
        byte  = bitSHLN(bitAnd(DECODE_TABLE[encodedBytes[i+4]], 1), 7);
        byte2 = bitSHLN(DECODE_TABLE[encodedBytes[i+5]], 2);
        byte3 = bitSHRN(DECODE_TABLE[encodedBytes[i+6]], 3);
        decodedBytes.write(bitOr(bitOr(byte, byte2), byte3));
        if (encodedBytes[i+7] == 61) break;
        byte  = bitSHLN(bitAnd(DECODE_TABLE[encodedBytes[i+6]], 7), 5);
        byte2 = DECODE_TABLE[encodedBytes[i+7]];
        decodedBytes.write(bitOr(byte, byte2));
    }
    return decodedBytes.toByteArray();
}

// "Hello!" is 6 bytes -> hits the ByteBuffer zero-padding branch (6 % 5 != 0).
// RFC 4648 base32("Hello!") = "JBSWY3DPEE======".
assert("base32encode('Hello!')", base32encode(javaCast("string","Hello!").getBytes()), "JBSWY3DPEE======");
assert("base32 round-trip recovers text", charsetEncode(base32decode("JBSWY3DPEE======"), "utf-8"), "Hello!");

// 5-byte input -> even multiple, no padding branch.
assert("base32encode('12345')", base32encode(javaCast("string","12345").getBytes()), "GEZDGNBV");
assert("base32 round-trip 5-byte", charsetEncode(base32decode("GEZDGNBV"), "utf-8"), "12345");

suiteEnd();
</cfscript>
