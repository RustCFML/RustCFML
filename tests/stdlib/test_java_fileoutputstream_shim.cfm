<cfscript>
suiteBegin("java.io.FileOutputStream shim");

// This shim backs Preside's chunked asset uploader
// (system/services/chunkedUpload/ChunkedUploadService.cfc), which assembles the
// uploaded chunks by streaming them to disk rather than reading the whole file
// into memory:
//
//     fos = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", tempFile ) );
//     for ( i=1; i<=totalChunks; i++ ) { fos.write( FileReadBinary( chunkFile ) ); }
//     fos.close();
//
// Before the shim existed this threw and no chunked upload could complete.

tmp = getTempDirectory() & "/fosshim_" & getTickCount() & "/";
directoryCreate( tmp );

// --- Preside's assembly shape: sequential write() calls append ---
totalChunks = 4;
expected    = "";
for ( i=1; i<=totalChunks; i++ ) {
	part = repeatString( chr( 64 + i ), 1000 );
	fileWrite( tmp & "chunk_" & i & ".bin", part );
	expected &= part;
}

assembled = tmp & "assembled.tmp";
fos = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", assembled ) );
try {
	for ( i=1; i<=totalChunks; i++ ) {
		fos.write( fileReadBinary( tmp & "chunk_" & i & ".bin" ) );
	}
} finally {
	fos.close();
}

assertTrue( "assembled file exists", fileExists( assembled ) );
assert( "assembled size is the sum of all chunks", getFileInfo( assembled ).size, len( expected ) );
assert( "assembled bytes are the chunks in order", fileRead( assembled ), expected );

// --- init() truncates, matching `new FileOutputStream(path)` on the JVM ---
fos2 = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", assembled ) );
fos2.write( toBinary( toBase64( "xy" ) ) );
fos2.close();
assert( "re-init truncates rather than appending", getFileInfo( assembled ).size, 2 );

// --- append=true preserves existing content ---
fos3 = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", assembled ), true );
fos3.write( toBinary( toBase64( "z" ) ) );
fos3.close();
assert( "append ctor preserves existing bytes", fileRead( assembled ), "xyz" );

// --- write(int) writes the low 8 bits ---
single = tmp & "single.bin";
fos4 = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", single ) );
fos4.write( 65 );
fos4.write( 66 );
fos4.close();
assert( "write(int) appends one byte per call", fileRead( single ), "AB" );

// --- write(byte[], off, len) writes just the range ---
ranged = tmp & "ranged.bin";
fos5 = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", ranged ) );
fos5.write( toBinary( toBase64( "ABCDEF" ) ), 2, 3 );
fos5.close();
assert( "write(byte[],off,len) writes only the range", fileRead( ranged ), "CDE" );

// --- flush() is callable and harmless ---
fos6 = createObject( "java", "java.io.FileOutputStream" ).init( javacast( "string", tmp & "flushed.bin" ) );
fos6.write( toBinary( toBase64( "q" ) ) );
fos6.flush();
fos6.close();
assert( "flush() then close() leaves the written byte", fileRead( tmp & "flushed.bin" ), "q" );

directoryDelete( tmp, true );

suiteEnd();
</cfscript>
