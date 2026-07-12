<cfscript>
suiteBegin("File I/O Functions");

tempDir = getTempDirectory();
assertTrue("getTempDirectory not empty", len(tempDir) > 0);

baseFile = tempDir & "rustcfml_test_" & createUUID() & ".txt";

// --- fileWrite / fileExists / fileRead ---
fileWrite(baseFile, "hello");
assertTrue("fileWrite creates file", fileExists(baseFile));
assert("fileRead returns content", fileRead(baseFile), "hello");

// --- fileAppend ---
fileAppend(baseFile, " world");
assert("fileAppend adds content", fileRead(baseFile), "hello world");

// --- fileDelete ---
fileDelete(baseFile);
assertFalse("fileDelete removes file", fileExists(baseFile));

// --- directoryExists ---
assertTrue("directoryExists on tempDir", directoryExists(tempDir));

// --- directoryCreate / directoryDelete ---
testDir = tempDir & "rustcfml_testdir_" & createUUID();
directoryCreate(testDir);
assertTrue("directoryCreate creates dir", directoryExists(testDir));
directoryDelete(testDir);
assertFalse("directoryDelete removes dir", directoryExists(testDir));

// --- fileCopy ---
srcFile = tempDir & "rustcfml_test_" & createUUID() & ".txt";
copyFile = tempDir & "rustcfml_test_" & createUUID() & ".txt";
fileWrite(srcFile, "copy me");
fileCopy(srcFile, copyFile);
assertTrue("fileCopy target exists", fileExists(copyFile));
assert("fileCopy content matches", fileRead(copyFile), "copy me");

// cleanup
fileDelete(srcFile);
fileDelete(copyFile);

// --- fileMove ---
moveFrom = tempDir & "rustcfml_test_" & createUUID() & ".txt";
moveTo = tempDir & "rustcfml_test_" & createUUID() & ".txt";
fileWrite(moveFrom, "move me");
fileMove(moveFrom, moveTo);
assertFalse("fileMove source gone", fileExists(moveFrom));
assertTrue("fileMove target exists", fileExists(moveTo));
assert("fileMove content preserved", fileRead(moveTo), "move me");

// cleanup
fileDelete(moveTo);

// --- expandPath ---
expanded = expandPath(".");
assertTrue("expandPath returns something", len(expanded) > 0);

// --- fileWrite of BINARY must round-trip (regression: was writing the
//     "<Binary>" placeholder string, corrupting Preside FileSystemStorageProvider) ---
binFile = tempDir & "rustcfml_bin_" & createUUID() & ".bin";
origBin = toBinary(toBase64("bytes-with-nulls-#chr(0)##chr(1)##chr(2)#-end"));
fileWrite(binFile, origBin);
readBin = fileReadBinary(binFile);
assertTrue("fileWrite binary: readback isBinary", isBinary(readBin));
assert("fileWrite binary round-trips", toBase64(readBin), toBase64(origBin));

// fileAppend of binary must also append raw bytes, not the placeholder
appendBin = toBinary(toBase64("-more-#chr(3)##chr(4)#"));
origLen = len(origBin);
appendLen = len(appendBin);
fileAppend(binFile, appendBin);
combined = fileReadBinary(binFile);
assertTrue("fileAppend binary: readback isBinary", isBinary(combined));
assert("fileAppend binary grows by appended byte count",
       len(combined), origLen + appendLen);
fileDelete(binFile);

suiteEnd();
</cfscript>
