<cfscript>
suiteBegin("directoryList");

// Create temp test structure
tmpDir = getTempDirectory() & "rustcfml_dirlist_test_" & createUUID();
directoryCreate(tmpDir);
directoryCreate(tmpDir & "/subdir1");
directoryCreate(tmpDir & "/subdir2");
fileWrite(tmpDir & "/file1.txt", "hello");
fileWrite(tmpDir & "/file2.cfm", "world");
fileWrite(tmpDir & "/subdir1/nested.txt", "nested");

// Default: list all (files AND directories), non-recursive, path mode
all = directoryList(tmpDir);
assertTrue("default lists files and dirs", arrayLen(all) >= 4); // subdir1, subdir2, file1.txt, file2.cfm

// Check directories are included
hasDir = false;
for (item in all) {
    if (find("subdir1", item)) hasDir = true;
}
assertTrue("directories included in results", hasDir);

// Non-recursive should not include nested files
hasNested = false;
for (item in all) {
    if (find("nested.txt", item)) hasNested = true;
}
assertFalse("non-recursive excludes nested", hasNested);

// Recursive
recursive = directoryList(tmpDir, true);
foundNested = false;
for (item in recursive) {
    if (find("nested.txt", item)) foundNested = true;
}
assertTrue("recursive includes nested files", foundNested);

// Name mode
names = directoryList(tmpDir, false, "name");
hasFileName = false;
for (item in names) {
    if (item == "file1.txt") hasFileName = true;
}
assertTrue("name mode returns filenames", hasFileName);

// Filter by extension - should only return matching files, not dirs
cfmOnly = directoryList(tmpDir, false, "name", "*.cfm");
assert("filter returns matching files", arrayLen(cfmOnly), 1);
assert("filter matches correct file", cfmOnly[1], "file2.cfm");

// Non-glob filter applies to BOTH files and directories (Lucee/ACF), but
// recursion still descends into every subdir. Regression: a literal filter
// like "ModuleConfig.cfc" used to leak every directory (broke TestBox module
// discovery). Set up Target.cfc at root + nested, plus non-matching dirs.
fileWrite(tmpDir & "/Target.cfc", "x");
fileWrite(tmpDir & "/subdir1/Target.cfc", "x");
recTargets = directoryList(tmpDir, true, "name", "Target.cfc");
assert("literal filter applies to dirs too (recursive)", arrayLen(recTargets), 2);
for (item in recTargets) {
    assert("only Target.cfc entries returned", item, "Target.cfc");
}

// Glob patterns with a wildcard in the MIDDLE (not just start/end). Regression:
// matches_filter used a naive contains(replace(*,"")) that failed for mid-pattern
// stars, e.g. "jquery-2*.min.js" would not match "jquery-2.2.5-sec.min.js" (broke
// Preside/Sticker wildcard asset resolution). Lucee matches these.
fileWrite(tmpDir & "/jquery-2.2.5-sec.min.js", "x");
fileWrite(tmpDir & "/jquery-ui-1.11.4.min.js", "x");

midStar = directoryList(tmpDir, false, "name", "jquery-2*.min.js");
assert("mid-pattern star matches one file", arrayLen(midStar), 1);
assert("mid-pattern star matches correct file", midStar[1], "jquery-2.2.5-sec.min.js");

midStar2 = directoryList(tmpDir, false, "name", "jquery-*.min.js");
assert("mid-pattern star matches both jquery libs", arrayLen(midStar2), 2);

// Trailing/leading star still work
trailStar = directoryList(tmpDir, false, "name", "jquery-2*");
assert("trailing star matches", arrayLen(trailStar), 1);

// Single '?' wildcard matches exactly one character
fileWrite(tmpDir & "/ab1.log", "x");
fileWrite(tmpDir & "/ab12.log", "x");
qmark = directoryList(tmpDir, false, "name", "ab?.log");
assert("? matches exactly one char", arrayLen(qmark), 1);
assert("? matched correct file", qmark[1], "ab1.log");

// Pipe-delimited multi-filter (Lucee/ACF): match if ANY sub-pattern matches
multi = directoryList(tmpDir, false, "name", "*.min.js|ab?.log");
assert("pipe-delimited filter matches union", arrayLen(multi), 3);

// Each sub-pattern is TRIMMED before matching (Lucee parity). Regression: a
// stray space made the filter silently match nothing — Preside's Sticker passes
// an exact filename straight from an extension's StickerBundle.cfc, and a
// trailing space there ("/js/lib/x.min.js ") threw Sticker.missingAsset for a
// file that existed on disk.
exactTrail = directoryList(tmpDir, false, "name", "file2.cfm ");
assert("exact filter with trailing space trims", arrayLen(exactTrail), 1);
assert("exact filter with trailing space matched", exactTrail[1], "file2.cfm");

exactLead = directoryList(tmpDir, false, "name", " file2.cfm");
assert("exact filter with leading space trims", arrayLen(exactLead), 1);

globTrail = directoryList(tmpDir, false, "name", "*.cfm ");
assert("glob filter with trailing space trims", arrayLen(globTrail), 1);

pipeSpaced = directoryList(tmpDir, false, "name", "*.min.js | ab?.log ");
assert("pipe-delimited filter trims each sub-pattern", arrayLen(pipeSpaced), 3);

// Cleanup
directoryDelete(tmpDir, true);

suiteEnd();
</cfscript>
