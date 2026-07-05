<cfscript>
suiteBegin("len() char-count (multibyte)");

// ── len() is a CHARACTER count, not a byte count ────────────────────────────
// Regression: len() used String::len() (UTF-8 bytes), disagreeing with the
// char-indexed mid()/left()/right() and the java.util.regex Matcher shim. That
// desynced Preside's DynamicFindAndReplaceService (Right(src, Len(src)-charPos))
// and leaked the raw delayed-Sticker "<!--ds:...:ds-->" include marker into
// rendered pages whenever a dump's multibyte "▾" twisties preceded it.
tw   = chr( 9662 );          // ▾  U+25BE — 3 UTF-8 bytes, 1 char
eacc = chr( 233 );           // é  U+00E9 — 2 UTF-8 bytes, 1 char
assert( "len() counts chars not bytes (BMP)", len( "a" & tw & "b" ), 3 );
assert( "len() counts chars not bytes (latin1)", len( "caf" & eacc ), 4 );
assert( "member .len() counts chars not bytes", ( "a" & tw & "b" ).len(), 3 );
assert( "len() stays byte-accurate for pure ASCII", len( "hello" ), 5 );

// len() must agree with the char-based slice functions on the same string.
mb = "x" & tw & tw & "yz";    // 5 chars
assert( "len() agrees with mid()/right() indexing", len( mb ), 5 );
assert( "right(mb,2) is last two chars", right( mb, 2 ), "yz" );

// Reproduce Preside's splice math: Right(source, Len(source) - matchEnd) after a
// regex match. With a byte-based len() this over-read by the multibyte byte-gap
// and re-appended the tail of the matched region (the leaked ":ds-->" marker).
function spliceAfterMarker( source ) {
	p = createObject( "java", "java.util.regex.Pattern" ).compile( "MARK" );
	m = p.matcher( javaCast( "string", arguments.source ) );
	before = ""; after = "";
	if ( m.find() ) {
		before = mid( arguments.source, 1, m.start() );          // char-based
		after  = right( arguments.source, len( arguments.source ) - m.end() );
	}
	return before & "[R]" & after;
}
// "▾▾ MARK tail"  →  before="▾▾ ", after=" tail"  →  "▾▾ [R] tail"
assert(
	  "regex splice stays aligned across multibyte chars"
	, spliceAfterMarker( tw & tw & " MARK tail" )
	, tw & tw & " [R] tail"
);

suiteEnd();
</cfscript>
