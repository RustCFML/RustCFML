<cfscript>
suiteBegin("Existence memo invalidation");

// `fileExists()`/`directoryExists()` are served by a request-scoped memo that
// caches POSITIVES only. Which builtins drop that memo is decided by
// `builtin_may_remove_path`: only an op that can make an existing path STOP
// existing invalidates it. Creations and overwrites do not — after them the
// path still exists, so a cached positive is still correct, and a path that did
// not exist was never cached in the first place.
//
// These tests pin the risky half of that rule. The danger is a delete going
// unseen because the memo still holds a positive, so each case populates the
// memo FIRST, then runs the exempted creation builtins, then deletes — the
// exact ordering that would expose an over-narrow invalidation list.

tmp = getTempDirectory();
uid = createUUID();
a   = tmp & "rcfml_memo_a_" & uid & ".txt";
b   = tmp & "rcfml_memo_b_" & uid & ".txt";

// --- a delete is seen after the memo has been populated ---------------------
fileWrite(a, "x");
assertTrue("populate memo: a exists", fileExists(a));
fileDelete(a);
assertFalse("fileDelete invalidates the memo", fileExists(a));

// --- a delete is still seen after intervening NON-removing builtins ---------
// fileWrite / fileWriteLine / fileAppend / fileOpen / fileClose no longer drop
// the memo. If any of them were wrongly relied on to do so, the delete below
// would be masked by the stale positive cached above it.
fileWrite(a, "x");
assertTrue("populate memo again", fileExists(a));

fileWrite(b, "other");          // creation elsewhere — must not be needed
fileAppend(b, " more");         // append elsewhere
hw = fileOpen(b, "append");     // handle lifecycle (write end)
fileWriteLine(hw, "line");      // append-line through the handle
fileClose(hw);                  // ...the 835-calls-per-request offender
hr = fileOpen(b, "read");
fileClose(hr);

fileDelete(a);
assertFalse("delete seen despite intervening creations", fileExists(a));

// --- the creations themselves still read back correctly --------------------
// Negatives are never cached, so a freshly created path must report present
// immediately even though nothing invalidated anything.
c = tmp & "rcfml_memo_c_" & uid & ".txt";
assertFalse("c absent before write", fileExists(c));
fileWrite(c, "new");
assertTrue("creation is visible with no invalidation", fileExists(c));
fileDelete(c);

// --- same contract for directories -----------------------------------------
d = tmp & "rcfml_memo_d_" & uid;
directoryCreate(d);
assertTrue("populate memo: directory exists", directoryExists(d));
fileWrite(b, "touch");          // exempted creation
directoryDelete(d);
assertFalse("directoryDelete invalidates the memo", directoryExists(d));

// --- fileMove removes its SOURCE, so it must still invalidate ---------------
e = tmp & "rcfml_memo_e_" & uid & ".txt";
f = tmp & "rcfml_memo_f_" & uid & ".txt";
fileWrite(e, "moveme");
assertTrue("populate memo: move source exists", fileExists(e));
fileMove(e, f);
assertFalse("fileMove invalidates for its source", fileExists(e));
assertTrue("fileMove destination exists", fileExists(f));

// cleanup
if (fileExists(b)) { fileDelete(b); }
if (fileExists(f)) { fileDelete(f); }

suiteEnd();
</cfscript>
