<cfscript>
suiteBegin( "File handles: fileOpen / fileReadLine / fileIsEof / fileSeek" );

dir = getTempDirectory() & "rcfml_fh_" & createUUID();
directoryCreate( dir );
f = dir & "/lines.txt";

// write mode creates the file at open time (known-issues #49), before any write
h = fileOpen( f, "write" );
assertTrue( "write-mode fileOpen creates the file immediately", fileExists( f ) );
fileWriteLine( h, "first" );
fileWriteLine( h, "second" );
fileWrite( h, "third" );
fileClose( h );
assert( "handle writes land in order", fileRead( f ), "first" & chr(10) & "second" & chr(10) & "third" );

// read mode: sequential lines, no terminators, eof only after the last line
h = fileOpen( f, "read" );
assertFalse( "not eof at open", fileIsEof( h ) );
assert( "line 1", fileReadLine( h ), "first" );
assert( "line 2", fileReadLine( h ), "second" );
assertFalse( "not eof before the last line", fileIsEof( h ) );
assert( "line 3 (no trailing newline)", fileReadLine( h ), "third" );
assertTrue( "eof after the last line", fileIsEof( h ) );
fileClose( h );

// a line loop reads every line exactly once
h = fileOpen( f, "read" );
n = 0;
while ( !fileIsEof( h ) ) { fileReadLine( h ); n++; }
fileClose( h );
assert( "line loop visits each line once", n, 3 );

// append mode adds to the end
h = fileOpen( f, "append" );
fileWriteLine( h, "" );
fileWriteLine( h, "fourth" );
fileClose( h );
assert( "append mode", listLast( fileRead( f ), chr(10) ), "fourth" );

// fileRead on a handle: n bytes, then the rest; fileSeek / fileSkipBytes reposition
h = fileOpen( f, "read" );
assert( "fileRead(handle, n) reads n chars", fileRead( h, 5 ), "first" );
fileSkipBytes( h, 1 );
assert( "fileSkipBytes then read", fileRead( h, 6 ), "second" );
fileSeek( h, 0 );
assert( "fileSeek(0) rewinds", fileReadLine( h ), "first" );
fileClose( h );

// the handle struct keeps the keys code inspects
h = fileOpen( f, "read" );
assert( "handle.mode", h.mode, "read" );
assert( "handle.filename", h.filename, "lines.txt" );
assertTrue( "handle.filepath ends with the file", right( h.filepath, 9 ) == "lines.txt" );
fileClose( h );

// opening a missing file for reading throws
assertThrows( "fileOpen read on a missing file throws", function() { fileOpen( dir & "/nope.txt", "read" ); } );

// scale: reading 2,000 lines is linear, not quadratic
big = dir & "/big.txt";
h = fileOpen( big, "write" );
for ( i = 1; i <= 2000; i++ ) { fileWriteLine( h, "line " & i ); }
fileClose( h );
t = getTickCount();
h = fileOpen( big, "read" );
c = 0;
while ( !fileIsEof( h ) ) { fileReadLine( h ); c++; }
fileClose( h );
assert( "2,000-line loop count", c, 2000 );
assertTrue( "2,000-line loop finishes in well under a second (" & ( getTickCount() - t ) & " ms)", ( getTickCount() - t ) < 1000 );

directoryDelete( dir, true );
suiteEnd();
</cfscript>
