<cfscript>
suiteBegin("getTempDirectory() ends with a separator (GH 380)");

// Lucee and Adobe CF both return the temp path WITH a trailing separator, and
// the ubiquitous `getTempDirectory() & name` join depends on it. Without it the
// join produced "/tmpname" — a path at the FILESYSTEM ROOT — so every following
// directoryCreate/fileWrite failed with a permission error. The bug was
// invisible on macOS, whose TMPDIR happens to end in a slash already; only
// Linux (a bare "/tmp") showed it.

tmp = getTempDirectory();

assert("the temp directory is not empty", len(tmp) gt 0, true);
assert("it ends with a path separator",
	reFind("[\\/]$", tmp) gt 0, true);
assert("it does not end with two separators",
	reFind("[\\/][\\/]$", tmp) gt 0, false);

// The join it exists to serve: the result must live INSIDE the temp directory,
// not alongside it.
joined = getTempDirectory() & "rustcfml-sep-probe";
assert("concatenation lands inside the temp directory",
	left(joined, len(tmp)), tmp);
assert("concatenation does not fuse the last segment onto the name",
	listLast(joined, "/\"), "rustcfml-sep-probe");

// And it has to be a directory that actually exists, or the join is moot.
assert("the temp directory exists", directoryExists(tmp), true);

suiteEnd();
</cfscript>
