<cfscript>
suiteBegin( "File handles: fileOpen / fileReadLine / fileIsEof / fileSeek" );

dir = getTempDirectory() & "rcfml_fh_" & createUUID();
directoryCreate( dir );
f = dir & "/lines.txt";

// Lucee 7.0/7.1: a write-mode open does not touch the file; the FIRST write
// creates (or truncates) it. fileExists sees it as soon as that write happens.
h = fileOpen( f, "write" );
assertFalse( "write-mode fileOpen does not create the file yet", fileExists( f ) );
fileWriteLine( h, "first" );
assertTrue( "the first write creates the file", fileExists( f ) );
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

// opening for write and closing without writing leaves an existing file intact
keep = dir & "/keep.txt";
fileWrite( keep, "old" );
h = fileOpen( keep, "write" );
assert( "a write-mode open does not truncate", fileRead( keep ), "old" );
fileClose( h );
assert( "closing an unwritten handle keeps the contents", fileRead( keep ), "old" );
h = fileOpen( dir & "/never.txt", "write" );
fileClose( h );
assertFalse( "an unwritten write handle creates nothing", fileExists( dir & "/never.txt" ) );

// append mode adds to the end
h = fileOpen( f, "append" );
fileWriteLine( h, "" );
fileWriteLine( h, "fourth" );
fileClose( h );
assert( "append mode", listLast( fileRead( f ), chr(10) ), "fourth" );

// fileRead on a handle: n bytes, then the rest; fileSeek / fileSkipBytes
// reposition. fileSeek needs fileOpen( …, seekable=true ), as on Lucee.
h = fileOpen( f, "read" );
assertThrows( "fileSeek on a non-seekable handle throws", function() { fileSeek( h, 0 ); } );
fileClose( h );
h = fileOpen( f, "read", "utf-8", true );
assert( "fileRead(handle, n) reads n chars", fileRead( h, 5 ), "first" );
fileSkipBytes( h, 1 );
assert( "fileSkipBytes then read", fileRead( h, 6 ), "second" );
fileSeek( h, 0 );
assert( "fileSeek(0) rewinds", fileReadLine( h ), "first" );
fileClose( h );

// the handle struct carries Lucee's keys: `path` is the DIRECTORY, `name` the
// file, plus mode/status/size/lastmodified; `status` becomes "close" on close
h = fileOpen( f, "read" );
assert( "handle.mode", h.mode, "read" );
assert( "handle.name", h.name, "lines.txt" );
assert( "handle.path is the directory", h.path & "/" & h.name, f );
assert( "handle.status while open", h.status, "open" );
assert( "handle.size", h.size, len( fileRead( f ) ) );
assertTrue( "handle.lastmodified is a date", isDate( h.lastmodified ) );
fileClose( h );
assert( "handle.status after close", h.status, "close" );

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
