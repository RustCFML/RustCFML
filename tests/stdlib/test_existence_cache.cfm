<cfscript>
// Two-layer existence cache behind fileExists()/directoryExists() (GH #299,
// known-issues §45). BOTH answers are cached — positive and negative — with
// positives invalidated per-path by the engine's own writes and negatives
// retired wholesale by a generation bump whenever anything may have created a
// path. So each probe below must reflect the tree as it is right then, not as an
// earlier probe in the same request saw it, in EITHER direction.
suiteBegin("Existence cache invalidation");

tmp = getTempDirectory();
f = tmp & "rustcfml_exists_" & createUUID() & ".txt";
d = tmp & "rustcfml_existsdir_" & createUUID();

// A cached negative must not make a later creation invisible — repeated probing
// of a missing path is exactly what seeds one (the `while (!fileExists(x))`
// poll pattern), so probe twice before creating.
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

// The SCRIPT form of the file/directory tags reaches the dispatcher under the
// TAG's own name, so it misses the file*/directory* prefix test that covers
// fileDelete()/directoryDelete() — a path deleted this way used to stay
// memoised as present. (The tag forms lower to those BIFs, covered above.)
tf = tmp & "rustcfml_exists_tag_" & createUUID() & ".txt";
cffile( action="write", file=tf, output="tagged" );
assertTrue("script-form cffile write: file exists (memoised)", fileExists(tf));
cffile( action="delete", file=tf );
assertFalse("script-form cffile delete invalidates the memoised positive", fileExists(tf));

td = tmp & "rustcfml_exists_tagdir_" & createUUID();
cfdirectory( action="create", directory=td );
assertTrue("script-form cfdirectory create: dir exists (memoised)", directoryExists(td));
cfdirectory( action="delete", directory=td );
assertFalse("script-form cfdirectory delete invalidates the memoised positive", directoryExists(td));

// An external program invoked by cfexecute can delete a memoised path behind
// the engine's back, so the whole memo has to go across the call.
xf = tmp & "rustcfml_exists_exec_" & createUUID() & ".txt";
fileWrite(xf, "bye");
assertTrue("cfexecute target exists (memoised)", fileExists(xf));
</cfscript>
<cfexecute name="/bin/rm" arguments="#xf#" timeout="10" />
<cfscript>
assertFalse("cfexecute invalidates the memoised positive", fileExists(xf));

// ---------------------------------------------------------------------------
// Creators that are VM-INTERCEPTED rather than dispatched as plain builtins.
//
// These never reach the builtin dispatcher, so a per-BIF "may create a path"
// predicate placed there cannot see them — which is exactly how `cfdump
// output=` regressed: the negative probe below cached "absent", cfdump then
// wrote the file, and fileExists() went on denying it for the rest of the
// request. Each case probes FIRST (seeding a negative), then creates, then
// re-probes.
// ---------------------------------------------------------------------------
df = tmp & "rustcfml_exists_dump_" & createUUID() & ".html";
assertFalse("cfdump target absent: probe 1", fileExists(df));
assertFalse("cfdump target absent: probe 2 (negative now cached)", fileExists(df));
writeDump( var={ a=1, b="two" }, output=df );
assertTrue("cfdump output= creation retires the cached negative", fileExists(df));
fileDelete(df);
assertFalse("cleanup: dump target removed", fileExists(df));

// NOTE: `fileOpen( f, "write" )` would be the other intercepted creator to pin
// here, but in RustCFML it does not create the file at all — verified outside the
// existence cache entirely (`directoryList` does not see it and it is absent on
// disk), so it is a separate divergence from Lucee rather than anything this
// cache can get wrong. See docs/known-issues.md §49.

// A cfthread has its own VM and its own writes; joining it is a yield point, so
// a negative cached before the join must not survive it.
tf2 = tmp & "rustcfml_exists_thread_" & createUUID() & ".txt";
assertFalse("thread target absent before start", fileExists(tf2));
thread name="existsWriter" target=tf2 {
	fileWrite( attributes.target, "written by thread" );
}
threadJoin("existsWriter");
assertTrue("threadJoin retires the cached negative", fileExists(tf2));
fileDelete(tf2);

suiteEnd();
</cfscript>
