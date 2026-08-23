<cfscript>
suiteBegin("urlEncode / urlEncodedFormat / encodeForURL are three distinct encoders (GH ##336)");

// These three functions disagree on the space AND on which punctuation
// survives, so each needs its own character policy. Every value below was
// measured character-by-character against Lucee 7.1.0.204 (box server,
// 2026-08-23) and cross-checked against Lucee's source:
//
//   urlEncode        URLEncode.java       — bare java.net.URLEncoder
//   urlEncodedFormat URLEncodedFormat.java — URLEncoder, then `+`->%20, then
//                                            escape * - . _
//   encodeForURL     ESAPI extension       — RFC 3986 unreserved set
//
// The `*` and `~` rows are the ones that make encodeForURL its own policy
// rather than an alias of either neighbour: it is the ONLY one that escapes
// `*`, and the ONLY one that leaves `~` alone.
//
// encodeForURL comes from a Lucee *extension*, not core, so its rows are
// live-measured only and could move with the extension version.

// ── space: the row GH #283 got backwards
assert( "space: urlEncode is form encoding",         urlEncode( " " ),        "+"   );
assert( "space: urlEncodedFormat percent-escapes",   urlEncodedFormat( " " ), "%20" );
assert( "space: encodeForURL percent-escapes",       encodeForURL( " " ),     "%20" );

// ── the four "safe" specials java.net.URLEncoder leaves alone
assert( "asterisk: urlEncode leaves it",             urlEncode( "*" ),        "*"   );
assert( "asterisk: urlEncodedFormat escapes it",     urlEncodedFormat( "*" ), "%2A" );
assert( "asterisk: encodeForURL escapes it",         encodeForURL( "*" ),     "%2A" );

assert( "hyphen: urlEncode leaves it",               urlEncode( "-" ),        "-"   );
assert( "hyphen: urlEncodedFormat escapes it",       urlEncodedFormat( "-" ), "%2D" );
assert( "hyphen: encodeForURL leaves it",            encodeForURL( "-" ),     "-"   );

assert( "period: urlEncodedFormat escapes it",       urlEncodedFormat( "." ), "%2E" );
assert( "period: encodeForURL leaves it",            encodeForURL( "." ),     "."   );

assert( "underscore: urlEncodedFormat escapes it",   urlEncodedFormat( "_" ), "%5F" );
assert( "underscore: encodeForURL leaves it",        encodeForURL( "_" ),     "_"   );

// ── tilde: unreserved in RFC 3986, not in form encoding
assert( "tilde: urlEncode escapes it",               urlEncode( "~" ),        "%7E" );
assert( "tilde: urlEncodedFormat escapes it",        urlEncodedFormat( "~" ), "%7E" );
assert( "tilde: encodeForURL leaves it",             encodeForURL( "~" ),     "~"   );

// ── a CMS slug is mostly hyphens, underscores and dots, so this is the shape
//    that actually shows up in emitted URLs
assert( "slug: urlEncodedFormat escapes every special",
        urlEncodedFormat( "my-page_v2.html" ), "my%2Dpage%5Fv2%2Ehtml" );
assert( "slug: urlEncode leaves a slug alone",
        urlEncode( "my-page_v2.html" ), "my-page_v2.html" );
assert( "slug: encodeForURL leaves a slug alone",
        encodeForURL( "my-page_v2.html" ), "my-page_v2.html" );

// ── a literal plus must never survive, whichever encoder ran
assert( "literal plus: urlEncode",         urlEncode( "a b+c" ),        "a+b%2Bc"   );
assert( "literal plus: urlEncodedFormat",  urlEncodedFormat( "a b+c" ), "a%20b%2Bc" );
assert( "literal plus: encodeForURL",      encodeForURL( "a b+c" ),     "a%20b%2Bc" );

// ── multi-byte input is UTF-8 percent-escaped identically by all three
assert( "utf-8: euro sign under urlEncodedFormat",   urlEncodedFormat( "€" ), "%E2%82%AC" );
assert( "utf-8: e-acute under encodeForURL",         encodeForURL( "é" ),     "%C3%A9"    );

suiteEnd();
</cfscript>
