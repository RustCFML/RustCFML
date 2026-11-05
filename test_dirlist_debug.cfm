<cfscript>
tempDir = getTempDirectory();
writeOutput("Temp dir: " & tempDir & "<br>");

// Create test files
testDir = tempDir & "test_dirlist_" & createUUID() & "/";
directoryCreate(testDir);
fileWrite(testDir & "a.txt", "test");
fileWrite(testDir & "b.txt", "test");

// Test 1: basic list
result1 = directoryList(testDir, false, "name");
writeOutput("Test 1 (name): " & arrayLen(result1) & " files<br>");
writeDump(result1);

// Test 2: with query
result2 = directoryList(testDir, false, "query");
writeOutput("<br>Test 2 (query): " & result2.recordCount & " files<br>");
writeDump(result2);

// Clean up
directoryDelete(testDir, true);
</cfscript>
