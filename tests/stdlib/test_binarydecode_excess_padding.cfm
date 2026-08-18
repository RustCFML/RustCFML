<cfscript>
// binaryDecode(..., "base64") and EXCESS padding. Lucee ignores surplus '='
// entirely; RustCFML agrees for partial-group excess but decodes a full
// spurious "====" quad appended to complete groups into a trailing NUL byte:
//
//   "QUJD"      -> "ABC" (3 bytes)            both engines
//   "QQ==="     -> 1 byte (one '=' surplus)   both engines
//   "QUJD===="  -> "ABC" (3) on Lucee, "ABC\0" (4) on RustCFML
//
// Repro class: jwt-cfml's base64UrlToBinary pads with
// `repeatString('=', 4 - (len % 4))` — i.e. FOUR '=' when the length is
// already a multiple of 4, which is exactly the whole-quad shape. On Lucee
// that lib bug is invisible; on RustCFML the NUL corrupts the decoded JWT
// header/payload JSON ("Invalid JSON: trailing characters"), breaking token
// decode on the surface the v0.606.0 crypto shims enabled. (titan fixed the
// lib's padding math; this pins the decoder-side leniency contract.)

suiteBegin("binaryDecode base64: excess padding is ignored, never decoded into bytes");

function decoded(s) {
    return charsetEncode(binaryDecode(s, "base64"), "utf-8");
}

assert( "control: exact-length group decodes clean", decoded("QUJD"), "ABC" );
assert( "control: exact-length group byte count", len(decoded("QUJD")), 3 );
assert( "partial excess (one surplus '=') is ignored", len(decoded("QQ===")), 1 );

wholeQuad = decoded("QUJD====");
assert( "whole-quad excess padding decodes to the same 3 bytes", len(wholeQuad), 3 );
assert( "no trailing NUL is materialised (saw last byte: " & asc(right(wholeQuad, 1)) & ")",
    wholeQuad, "ABC" );

suiteEnd();
</cfscript>
