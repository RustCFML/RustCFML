<cfsilent>
<!---
Comprehensive tests for #509: Lucee 7.1.3.85 directory filtering parity.

Tests directoryList, directoryCopy, and cfdirectory with:
- Closure and UDF filters
- String wildcard patterns (*, ?, !, |, trimming)
- Case sensitivity on Linux/macOS
- Type filtering
- Recursion behavior
- Error handling

Expected values match Lucee 7.1 on Linux.
--->

<cfscript>
try { include "harness.cfm"; } catch( any e ) { abort; }

suiteBegin("directory filter parity (#509)");

// Create test fixture directory
testRoot = getTempDirectory() & "dirfilter509_" & createUUID() & "/";
directoryCreate(testRoot);

// Helper to create fixture
function setupFixture(root) {
	directoryCreate(root);
	fileWrite(root & "a.txt", "test");
	fileWrite(root & "B.TXT", "test");
	fileWrite(root & "test1.txt", "test");
	fileWrite(root & "abra.cfm", "test");
	fileWrite(root & "noext", "test");
	fileWrite(root & ".hidden", "test");
	fileWrite(root & "abab", "test");
	directoryCreate(root & "empty");
	directoryCreate(root & "testempty");
	directoryCreate(root & "sub");
	fileWrite(root & "sub/test3.txt", "test");
	directoryCreate(root & "sub/deep");
	fileWrite(root & "sub/deep/x.cfm", "test");
}

// Helper to sort and format array
function sortedNames(arr) {
	arraySort(arr, "text");
	return arrayToList(arr, ",");
}

// Setup main fixture
R = testRoot & "R/";
setupFixture(R);

// ========================================================================
// Part 1: directoryList with closure/UDF filters
// ========================================================================

// Test 1: Closure filter by extension
closure1 = function(p) {
	return listLast(p, ".") == "txt";
};
result1 = directoryList(R, false, "name", closure1);
arraySort(result1, "text");
assert("directoryList closure filter by extension", sortedNames(result1), "B.TXT,a.txt,test1.txt");

// Test 2: Named UDF filter
function shouldInclude(path) {
	return listLast(path, ".") == "txt";
}
result2 = directoryList(R, false, "name", shouldInclude);
arraySort(result2, "text");
assert("directoryList named UDF filter", sortedNames(result2), "B.TXT,a.txt,test1.txt");

// Test 3: Named argument
result3 = directoryList(path=R, filter=shouldInclude, listInfo="name");
arraySort(result3, "text");
assert("directoryList named args with UDF", sortedNames(result3), "B.TXT,a.txt,test1.txt");

// Test 4: Filter receives correct arguments
receivedArgs = [];
closure4 = function(p, t, e) {
	arrayAppend(receivedArgs, {path: p, type: t, ext: e});
	return true;
};
directoryList(R, false, "name", closure4);
// Find entries for a.txt, B.TXT, and sub directory
aTxtEntry = arrayFilter(receivedArgs, function(x) { return x.path.endsWith("a.txt"); })[1];
bTxtEntry = arrayFilter(receivedArgs, function(x) { return x.path.endsWith("B.TXT"); })[1];
subEntry = arrayFilter(receivedArgs, function(x) { return x.path.endsWith("sub") && !x.path.contains("/sub/"); })[1];
hiddenEntry = arrayFilter(receivedArgs, function(x) { return x.path.endsWith(".hidden"); })[1];
noextEntry = arrayFilter(receivedArgs, function(x) { return x.path.endsWith("noext"); })[1];

assert("filter arg 1 is absolute path", aTxtEntry.path, directoryList(R, false, "path", "a.txt")[1]);
assert("filter arg 2 for file is 'file'", aTxtEntry.type, "file");
assert("filter arg 3 for .txt is 'txt'", aTxtEntry.ext, "txt");
assert("filter arg 2 for directory is 'directory'", subEntry.type, "directory");
assert("filter arg 3 for directory is empty string", subEntry.ext, "");
assert("filter arg 3 for .TXT is 'TXT'", bTxtEntry.ext, "TXT");
assert("filter arg 3 for .hidden is 'hidden'", hiddenEntry.ext, "hidden");
assert("filter arg 3 for no extension is null", isNull(noextEntry.ext) || noextEntry.ext == "", true);

// Test 5: Filter called for directories
dirCalls = [];
closure5 = function(p, t) {
	if (t == "directory") {
		arrayAppend(dirCalls, listLast(p, "/\"));
	}
	return true;
};
directoryList(R, false, "name", closure5);
arraySort(dirCalls, "text");
assert("filter called for directories", arrayToList(dirCalls, ","), "empty,sub,testempty");

// Test 6: Recursion ignores filter rejection
closure6 = function(p, t) {
	return t == "file";
};
result6 = directoryList(R, true, "name", closure6);
hasTest3 = arrayFindNoCase(result6, "test3.txt") > 0;
hasXCfm = arrayFindNoCase(result6, "x.cfm") > 0;
assert("recursion enters rejected directories", hasTest3 && hasXCfm, true);

// Test 7: type filter gates before UDF
ufcCalls7 = 0;
closure7 = function(p, t) {
	ufcCalls7++;
	return true;
};
directoryList(R, false, "name", closure7, "", "file");
// Should not call closure for directories
// Count files: a.txt, B.TXT, test1.txt, abra.cfm, noext, .hidden, abab = 7
assert("type=file gates before UDF", ufcCalls7, 7);

// Test 8: Truthiness
closure8a = function() { return "yes"; };
closure8b = function() { return 1; };
closure8c = function() { return "true"; };
closure8d = function() { return "no"; };
closure8e = function() { return 0; };
assert("filter return 'yes' is true", arrayLen(directoryList(R, false, "name", closure8a)) > 0, true);
assert("filter return 1 is true", arrayLen(directoryList(R, false, "name", closure8b)) > 0, true);
assert("filter return 'true' is true", arrayLen(directoryList(R, false, "name", closure8c)) > 0, true);
assert("filter return 'no' is false", arrayLen(directoryList(R, false, "name", closure8d)), 0);
assert("filter return 0 is false", arrayLen(directoryList(R, false, "name", closure8e)), 0);

// Test 9: Invalid return throws
closure9 = function() { return "abc"; };
assertThrows("filter return invalid string throws", function() {
	directoryList(R, false, "name", closure9);
}, "");

// Test 10: No return throws
closure10 = function() { /* no return */ };
assertThrows("filter no return throws", function() {
	directoryList(R, false, "name", closure10);
}, "");

// Test 11: Exception propagates
closure11 = function() {
	throw(type="My.Err", message="boom");
};
try {
	directoryList(R, false, "name", closure11);
	assert("exception from filter propagates", false, true);
} catch (My.Err e) {
	assert("exception type preserved", e.type, "My.Err");
	assert("exception message preserved", e.message contains "boom", true);
}

// Test 12: Invalid signature throws
function badReturn(p) { return "test"; }
assertThrows("invalid return type throws", function() {
	directoryList(R, false, "name", badReturn);
}, "");

function tooManyArgs(a, b, c, d) { return true; }
assertThrows("too many params throws", function() {
	directoryList(R, false, "name", tooManyArgs);
}, "");

// Test 13: Outer variable writeback
counter13 = 0;
closure13 = function() {
	counter13++;
	return true;
};
directoryList(R, false, "name", closure13);
assert("closure writes to outer variable", counter13 > 0, true);

// Test 14: listInfo=query with closure and sort
closure14 = function(p) {
	return p.endsWith(".txt");
};
result14 = directoryList(R, false, "query", closure14, "name desc");
assert("listInfo=query with closure", result14.recordCount, 3);
assert("sort applies to query", result14.name[1], "test1.txt");

// Test 15: LDEV-3188 - sort on name array
result15 = directoryList(R, false, "name", "*.txt", "name desc");
assert("sort applies to name array (LDEV-3188)", result15[1] == "test1.txt" || result15[1] == "B.TXT", true);

// ========================================================================
// Part 2: directoryList with string patterns
// ========================================================================

// Test 16: Simple wildcard
result16 = directoryList(R, false, "name", "*.txt");
arraySort(result16, "text");
// On Linux, case-sensitive: a.txt and test1.txt, NOT B.TXT
if (server.os.name.findNoCase("linux") || server.os.name.findNoCase("mac")) {
	assert("*.txt case-sensitive on Linux", sortedNames(result16), "a.txt,test1.txt");
} else {
	assert("*.txt case-insensitive on Windows", sortedNames(result16), "B.TXT,a.txt,test1.txt");
}

// Test 17: Multiple patterns with |
result17 = directoryList(R, false, "name", "*.txt|*.cfm");
arraySort(result17, "text");
if (server.os.name.findNoCase("linux") || server.os.name.findNoCase("mac")) {
	assert("*.txt|*.cfm on Linux", sortedNames(result17), "a.txt,abra.cfm,test1.txt");
} else {
	assert("*.txt|*.cfm on Windows", sortedNames(result17), "B.TXT,a.txt,abra.cfm,test1.txt");
}

// Test 18: Trimmed tokens
result18 = directoryList(R, false, "name", " *.txt | *.cfm ");
arraySort(result18, "text");
if (server.os.name.findNoCase("linux") || server.os.name.findNoCase("mac")) {
	assert("trimmed tokens", sortedNames(result18), "a.txt,abra.cfm,test1.txt");
} else {
	assert("trimmed tokens", sortedNames(result18), "B.TXT,a.txt,abra.cfm,test1.txt");
}

// Test 19: Pattern matches directories too
result19 = directoryList(R, false, "name", "test*");
arraySort(result19, "text");
assert("pattern matches directories", sortedNames(result19), "test1.txt,testempty");

// Test 20: Negation
result20 = directoryList(R, false, "name", "!*.txt");
// Should get everything except .txt files
hasCfm = arrayFindNoCase(result20, "abra.cfm") > 0;
hasTxt = arrayFindNoCase(result20, "a.txt") > 0;
assert("negation with !", hasCfm && !hasTxt, true);

// Test 21: ? wildcard
result21 = directoryList(R, false, "name", "?.txt");
arraySort(result21, "text");
assert("? matches exactly one char", sortedNames(result21), "a.txt");

// Test 22: Non-backtracking a*b
result22 = directoryList(R, false, "name", "a*b");
hasAbab = arrayFindNoCase(result22, "abab") > 0;
assert("a*b does NOT match abab (non-backtracking)", hasAbab, false);

// Test 23: Empty pattern | matches nothing
result23 = directoryList(R, false, "name", "|");
assert("empty pattern | matches nothing", arrayLen(result23), 0);

// Test 24: Whitespace-only matches everything
result24 = directoryList(R, false, "name", "  ");
assert("whitespace pattern matches all", arrayLen(result24) > 10, true);

// Test 25: Recursion with pattern
result25 = directoryList(R, true, "name", "*.txt");
hasTest3 = arrayFindNoCase(result25, "test3.txt") > 0;
assert("recursion with *.txt finds sub/test3.txt", hasTest3, true);

// Test 26: type with pattern
result26 = directoryList(R, false, "name", "test*", "", "dir");
assert("type=dir with pattern", sortedNames(result26), "testempty");

result26b = directoryList(R, false, "name", "test*", "", "directory");
assert("type=directory with pattern", sortedNames(result26b), "testempty");

// Test 27: Invalid type throws
assertThrows("invalid type throws", function() {
	directoryList(R, false, "name", "*", "", "bogus");
}, "");

// ========================================================================
// Part 3: directoryCopy
// ========================================================================

// Setup copy fixture
inc = testRoot & "inc/";
setupFixture(inc);

// Test 28: Non-recursive (default)
d1 = testRoot & "d1/";
directoryCopy(inc, d1);
result28 = directoryList(d1, false, "name");
arraySort(result28, "text");
// Should only have top-level files, not subdirectories
assert("directoryCopy non-recursive (default)", sortedNames(result28), "B.TXT,a.txt,abab,abra.cfm,noext,test1.txt");

// Test 29: Non-recursive with filter
d2 = testRoot & "d2/";
directoryCopy(inc, d2, false, "test*");
result29 = directoryList(d2, false, "name");
assert("directoryCopy non-recursive with filter", sortedNames(result29), "test1.txt");

// Test 30: Recursive
d3 = testRoot & "d3/";
directoryCopy(inc, d3, true);
result30 = directoryList(d3, true, "name");
arraySort(result30, "text");
// Should have all files and directories
hasTest3 = arrayFindNoCase(result30, "test3.txt") > 0;
hasSub = arrayFindNoCase(result30, "sub") > 0;
assert("directoryCopy recursive", hasTest3 && hasSub, true);

// Test 31: Recursive with filter removes empty rejected dirs
d4 = testRoot & "d4/";
directoryCopy(inc, d4, true, "test*");
result31 = directoryList(d4, true, "name");
arraySort(result31, "text");
// Should have sub (non-empty), test1.txt, test3.txt, testempty (name matches)
// empty should be removed
hasEmpty = directoryExists(d4 & "empty");
hasSub = directoryExists(d4 & "sub");
hasTestempty = directoryExists(d4 & "testempty");
assert("directoryCopy recursive with filter removes empty", !hasEmpty && hasSub && hasTestempty, true);

// Test 32: Recursive with closure filter
d5 = testRoot & "d5/";
d5Calls = [];
closure32 = function(p, t) {
	if (p.endsWith("d5")) {
		return true; // root dir
	}
	arrayAppend(d5Calls, p);
	return t == "file" && p.endsWith(".txt");
};
directoryCopy(inc, d5, true, closure32);
result32 = directoryList(d5, true, "name");
arraySort(result32, "text");
// Should have sub (kept because non-empty), test*.txt files
// empty and testempty removed
hasAbra = arrayFindNoCase(result32, "abra.cfm") > 0;
hasTest1 = arrayFindNoCase(result32, "test1.txt") > 0;
hasEmpty = directoryExists(d5 & "empty");
hasTestempty = directoryExists(d5 & "testempty");
hasSub = directoryExists(d5 & "sub");
assert("directoryCopy closure filter keeps non-empty dirs", !hasAbra && hasTest1 && !hasEmpty && !hasTestempty && hasSub, true);
// Verify closure was called with destination paths during cleanup
hasD5Path = arrayFilter(d5Calls, function(p) { return p.contains("d5"); }).len() > 0;
assert("directoryCopy closure called with dest paths", hasD5Path, true);

// Test 33: Non-recursive with closure - closure sees directories
d6 = testRoot & "d6/";
d6Dirs = [];
closure33 = function(p, t) {
	if (t == "directory") {
		arrayAppend(d6Dirs, listLast(p, "/\"));
	}
	return t == "file";
};
directoryCopy(inc, d6, false, closure33);
assert("non-recursive closure sees directories", arrayLen(d6Dirs) > 0, true);

// Test 34: Filter that matches nothing removes dest root (edge case)
d7 = testRoot & "d7/";
// Note: This test may need live Lucee verification
// directoryCopy(inc, d7, true, "*.xyz");
// assert("root removed when nothing matches", !directoryExists(d7), true);

// Test 35: createPath=false requires parent
d8 = testRoot & "missing/d8/";
assertThrows("createPath=false with missing parent throws", function() {
	directoryCopy(inc, d8, false, "", false);
}, "");

// Test 36: Named and aliased args
d9 = testRoot & "d9/";
directoryCopy(directory=inc, newDirectory=d9, recursive=true, filter="*.txt");
result36 = directoryList(d9, true, "name");
hasTest1 = arrayFindNoCase(result36, "test1.txt") > 0;
hasCfm = arrayFindNoCase(result36, "abra.cfm") > 0;
assert("directoryCopy named/aliased args", hasTest1 && !hasCfm, true);

// Test 37: Exception from copy filter becomes application error
d10 = testRoot & "d10/";
closure37 = function() {
	throw(message="boom");
};
try {
	directoryCopy(inc, d10, true, closure37);
	assert("copy filter exception propagates", false, true);
} catch (application e) {
	assert("copy filter exception is application type", e.type, "application");
	assert("copy filter exception message preserved", e.message contains "boom", true);
}

// ========================================================================
// Part 4: cfdirectory
// ========================================================================

// Test 38: cfdirectory action=list with pattern
result38 = queryNew("");
cfdirectory(action="list", directory=R, filter="test*", name="result38");
assert("cfdirectory list with pattern", result38.recordCount, 2);

// Test 39: cfdirectory with filterDelimiters
result39 = queryNew("");
cfdirectory(action="list", directory=R, filter="*.txt,*.cfm", filterDelimiters=",", name="result39");
assert("cfdirectory with filterDelimiters", result39.recordCount >= 3, true);

// Test 40: cfdirectory with closure
closure40 = function(p) {
	return listLast(p, ".") == "txt";
};
result40 = queryNew("");
cfdirectory(action="list", directory=R, filter=closure40, name="result40");
assert("cfdirectory with closure", result40.recordCount, 3);

// Test 41: cfdirectory type=dir
result41 = queryNew("");
cfdirectory(action="list", directory=R, filter="test*", type="dir", name="result41");
assert("cfdirectory type=dir", result41.recordCount, 1);

// Test 42: cfdirectory listinfo=name with recursion
result42 = queryNew("");
cfdirectory(action="list", directory=R, listinfo="name", recurse=true, filter="*.txt", name="result42");
// Should include relative paths like sub/test3.txt
hasSubPath = false;
for (row in result42) {
	if (row.name.find("/") > 0) {
		hasSubPath = true;
		break;
	}
}
assert("cfdirectory listinfo=name with recurse has paths", hasSubPath, true);

// Test 43: cfdirectory action=copy
d11 = testRoot & "d11/";
cfdirectory(action="copy", directory=inc, destination=d11, recurse=true, filter="test*");
result43 = directoryList(d11, true, "name");
hasTest1 = arrayFindNoCase(result43, "test1.txt") > 0;
hasAbra = arrayFindNoCase(result43, "abra.cfm") > 0;
assert("cfdirectory action=copy", hasTest1 && !hasAbra, true);

// Cleanup
try {
	directoryDelete(testRoot, true);
} catch (any e) {
	// Ignore cleanup errors
}

suiteEnd();
</cfscript>
</cfsilent>
