<cfscript>
// Request-scoped existence memo behind fileExists()/directoryExists() (GH #299).
// Only POSITIVE answers are memoised, and every filesystem-mutating BIF drops
// the memo — so each probe below must reflect the tree as it is right then, not
// as an earlier probe in the same request saw it.
suiteBegin("Existence cache invalidation");

tmp = getTempDirectory();
f = tmp & "rustcfml_exists_" & createUUID() & ".txt";
d = tmp & "rustcfml_existsdir_" & createUUID();

// A negative is never cached: probing a missing path repeatedly must not make
// its later creation invisible (the `while (!fileExists(x))` poll pattern).
assertFalse("missing file: probe 1", fileExists(f));
assertFalse("missing file: probe 2", fileExists(f));
fileWrite(f, "one");
assertTrue("file becomes visible after write", fileExists(f));
assertTrue("file still visible when memoised", fileExists(f));

// A memoised positive must not survive the delete.
fileDelete(f);
assertFalse("delete invalidates the memoised positive", fileExists(f));

// Same for directories.
assertFalse("missing dir: probe 1", directoryExists(d));
assertFalse("missing dir: probe 2", directoryExists(d));
directoryCreate(d);
assertTrue("dir visible after create", directoryExists(d));
assertTrue("dir still visible when memoised", directoryExists(d));
directoryDelete(d);
assertFalse("directoryDelete invalidates the memoised positive", directoryExists(d));

// fileMove changes TWO paths — the memo must not keep the source alive.
src = tmp & "rustcfml_exists_src_" & createUUID() & ".txt";
dst = tmp & "rustcfml_exists_dst_" & createUUID() & ".txt";
fileWrite(src, "move me");
assertTrue("move source exists (memoised)", fileExists(src));
assertFalse("move target does not exist yet", fileExists(dst));
fileMove(src, dst);
assertFalse("move invalidates the source positive", fileExists(src));
assertTrue("move target now exists", fileExists(dst));

// fileCopy, then remove the copy.
copy = tmp & "rustcfml_exists_copy_" & createUUID() & ".txt";
fileCopy(dst, copy);
assertTrue("copy target exists", fileExists(copy));
fileDelete(copy);
assertFalse("copy target gone after delete", fileExists(copy));

// A rewrite of an existing (memoised) file leaves it existing, with new content.
fileWrite(dst, "rewritten");
assertTrue("rewritten file still exists", fileExists(dst));
assert("rewritten content is read back", fileRead(dst), "rewritten");
fileDelete(dst);
assertFalse("cleanup: target removed", fileExists(dst));

// The file/directory kinds are memoised separately — a directory is not a file
// and vice versa, no matter which probe ran first.
kd = tmp & "rustcfml_existskind_" & createUUID();
directoryCreate(kd);
kf = kd & "/probe.txt";
fileWrite(kf, "x");
assertTrue("directoryExists on the dir", directoryExists(kd));
assertFalse("fileExists FALSE on the dir (after dir probe)", fileExists(kd));
assertTrue("fileExists on the file", fileExists(kf));
assertFalse("directoryExists FALSE on the file (after file probe)", directoryExists(kf));
fileDelete(kf);
directoryDelete(kd);
assertFalse("cleanup: kind dir removed", directoryExists(kd));

suiteEnd();
</cfscript>
