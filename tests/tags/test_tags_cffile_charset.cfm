<cfscript>
suiteBegin("Tags: cffile / fileRead / fileWrite charset=");

// The engine had NO character-encoding layer: `<cffile charset=>` was dropped at
// lowering, fileRead/fileWrite/fileAppend ignored a charset argument, and
// charsetEncode/charsetDecode were pass-through no-ops — so everything was UTF-8
// whatever the caller asked for (docs known-issues §27).
// Every expectation below was probed against Lucee 7.0.4 with the text below:
// "a" (1 UTF-8 byte), U+00E9 (2 bytes) and U+20AC (3 bytes).
csDir = getTempDirectory() & "rcf_charset_" & createUUID() & "/";
directoryCreate( csDir );
csText = "a" & chr(233) & chr(8364);

function csFile( nm ) { return csDir & nm & ".txt"; }
function csBytes( path ) { return len( fileReadBinary( path ) ); }
function csCodes( str ) {
	var out = [];
	for ( var i = 1; i <= len( str ); i++ ) { arrayAppend( out, asc( mid( str, i, 1 ) ) ); }
	return arrayToList( out, "," );
}

// ---- byte counts prove the encoding was actually applied ---------------------
fileWrite( csFile( "u8" ),    csText, "utf-8"  );
fileWrite( csFile( "u16" ),   csText, "utf-16" );
fileWrite( csFile( "u16be" ), csText, "utf-16be" );
fileWrite( csFile( "u16le" ), csText, "utf-16le" );
fileWrite( csFile( "l1" ),    csText, "iso-8859-1" );
fileWrite( csFile( "cp" ),    csText, "windows-1252" );
fileWrite( csFile( "asc" ),   csText, "us-ascii" );

assert( "fileWrite utf-8 byte count", csBytes( csFile( "u8" ) ), 6 );
// utf-16 carries a 2-byte BOM; the BE/LE forms do not.
assert( "fileWrite utf-16 writes a BOM", csBytes( csFile( "u16" ) ), 8 );
assert( "fileWrite utf-16be byte count", csBytes( csFile( "u16be" ) ), 6 );
assert( "fileWrite utf-16le byte count", csBytes( csFile( "u16le" ) ), 6 );
// Single-byte encodings: one byte per character, unmappable ones become "?".
assert( "fileWrite iso-8859-1 byte count", csBytes( csFile( "l1" ) ), 3 );
assert( "fileWrite windows-1252 byte count", csBytes( csFile( "cp" ) ), 3 );
assert( "fileWrite us-ascii byte count", csBytes( csFile( "asc" ) ), 3 );

// ---- round-trips ------------------------------------------------------------
assert( "fileRead utf-8 round-trips", fileRead( csFile( "u8" ), "utf-8" ), csText );
assert( "fileRead utf-16 round-trips", fileRead( csFile( "u16" ), "utf-16" ), csText );
assert( "fileRead utf-16be round-trips", fileRead( csFile( "u16be" ), "utf-16be" ), csText );
assert( "fileRead utf-16le round-trips", fileRead( csFile( "u16le" ), "utf-16le" ), csText );
// windows-1252 can represent all three characters; latin-1 and ascii cannot, and
// the character lost on WRITE stays lost — it is a "?" on the way back.
assert( "fileRead windows-1252 round-trips", fileRead( csFile( "cp" ), "windows-1252" ), csText );
assert( "fileRead iso-8859-1 keeps the ? substitution", csCodes( fileRead( csFile( "l1" ), "iso-8859-1" ) ), "97,233,63" );
assert( "fileRead us-ascii keeps both ? substitutions", csCodes( fileRead( csFile( "asc" ), "us-ascii" ) ), "97,63,63" );

// ---- a BOM wins over the requested charset ----------------------------------
// This is what lets a utf-16 file be read by a caller who asks for utf-8 (or
// asks for nothing) — Lucee sniffs the BOM first.
assert( "a BOM overrides an explicit utf-8 read", fileRead( csFile( "u16" ), "utf-8" ), csText );
assert( "a BOM is honoured with no charset at all", fileRead( csFile( "u16" ) ), csText );
// Without a BOM there is nothing to sniff, so a utf-8 read of utf-16be bytes is
// mojibake with U+FFFD replacements — lossy, but never an error.
assert( "utf-16be read as utf-8 is lossy, not an error", csCodes( fileRead( csFile( "u16be" ), "utf-8" ) ), "0,97,0,65533,32,65533" );

// ---- append ----------------------------------------------------------------
fileAppend( csFile( "u16" ), csText, "utf-16" );
// Lucee appends the whole encoding INCLUDING a second BOM.
assert( "fileAppend charset= appends the full encoding, BOM included", csBytes( csFile( "u16" ) ), 16 );

// ---- charsetDecode / charsetEncode -----------------------------------------
assert( "charsetEncode(charsetDecode(x, utf-8), utf-8) round-trips",
	charsetEncode( charsetDecode( csText, "utf-8" ), "utf-8" ), csText );
assert( "charsetEncode(charsetDecode(x, utf-16), utf-16) round-trips",
	charsetEncode( charsetDecode( csText, "utf-16" ), "utf-16" ), csText );
// The encodings must actually differ — this is what a no-op charset argument
// could never show. (Comparing the decoded STRINGS would not: the utf-16 bytes
// start with a BOM, and a BOM wins over the charset asked for, so reading them
// back "as utf-8" legitimately returns the original text.)
assert( "charsetDecode utf-8 byte count", len( charsetDecode( csText, "utf-8" ) ), 6 );
assert( "charsetDecode utf-16 byte count includes the BOM", len( charsetDecode( csText, "utf-16" ) ), 8 );
assert( "charsetDecode iso-8859-1 byte count", len( charsetDecode( csText, "iso-8859-1" ) ), 3 );

// ---- an unknown charset is an error, not a silent UTF-8 fallback ------------
csErr = "";
try {
	fileWrite( csFile( "bogus" ), csText, "not-a-charset" );
} catch ( any e ) {
	csErr = e.message;
}
assertTrue( "an unknown charset on write raises an error", len( csErr ) > 0 );

csReadErr = "";
try {
	fileRead( csFile( "u8" ), "not-a-charset" );
} catch ( any e ) {
	csReadErr = e.message;
}
assertTrue( "an unknown charset on read raises an error", len( csReadErr ) > 0 );
</cfscript>

<!--- The tag form must forward charset= exactly as the BIFs receive it. Note the
      tag ALSO appends a line separator unless addNewLine="false" (Lucee-probed:
      `<cffile action="write" output="abc">` writes 4 bytes, `fileWrite()` writes
      3), so these byte counts carry one extra encoded newline — 2 bytes in
      UTF-16 — over the BIF counts above. --->
<cffile action="write" file="#csDir#tag16.txt" output="#csText#" charset="utf-16">
<cffile action="read" file="#csDir#tag16.txt" variable="tagBack" charset="utf-16">
<cfscript>
assert( "cffile action=write charset= reaches the file", csBytes( csDir & "tag16.txt" ), 10 );
assert( "cffile action=write appends a line separator", tagBack, csText & chr(10) );
</cfscript>

<cffile action="append" file="#csDir#tag16.txt" output="#csText#" charset="utf-16">
<cfscript>
assert( "cffile action=append charset= reaches the file", csBytes( csDir & "tag16.txt" ), 20 );
</cfscript>

<!--- addNewLine="false" suppresses it, on both actions. --->
<cffile action="write" file="#csDir#tagnl.txt" output="#csText#" charset="utf-16" addNewLine="false">
<cfscript>
assert( "cffile action=write addNewLine=false writes exact bytes", csBytes( csDir & "tagnl.txt" ), 8 );
assert( "cffile action=write addNewLine=false round-trips exactly",
	fileRead( csDir & "tagnl.txt", "utf-16" ), csText );
</cfscript>

<cffile action="append" file="#csDir#tagnl.txt" output="#csText#" charset="utf-16" addNewLine="false">
<cfscript>
assert( "cffile action=append addNewLine=false appends exact bytes", csBytes( csDir & "tagnl.txt" ), 16 );

// The fileWrite/fileAppend BIFs never add a separator — only the tag does.
fileWrite( csDir & "bifnl.txt", "abc" );
assert( "the fileWrite BIF adds no separator", csBytes( csDir & "bifnl.txt" ), 3 );

directoryDelete( csDir, true );
suiteEnd();
</cfscript>
