<cfscript>
suiteBegin("java.io.File two-argument (parent, child) constructor (GH 378)");

// `new File(parent, child)` resolved to the PARENT alone — the child argument
// was dropped silently, so the standard idiom operated on the directory instead
// of the file inside it. Nothing threw; the wrong path was simply used.

base = getDirectoryFromPath(getCurrentTemplatePath());
base = reReplace(base, "[\\/]$", "");

twoArg = createObject("java", "java.io.File").init(base, "backup.sql");
oneArg = createObject("java", "java.io.File").init(base);

assert("the child segment is appended, not dropped",
	listLast(twoArg.getPath(), "/\"), "backup.sql");

assert("the two-arg form does not resolve to the parent",
	twoArg.getPath() neq oneArg.getPath(), true);

assert("the result is the one-argument form of the joined path",
	twoArg.getPath(),
	createObject("java", "java.io.File").init(base & "/backup.sql").getPath());

// A File may stand in for the parent, which is how the idiom usually appears.
fromFileParent = createObject("java", "java.io.File").init(oneArg, "backup.sql");
assert("a File parent resolves the same as a string parent",
	fromFileParent.getPath(), twoArg.getPath());

// Java resolves the child AGAINST the parent, so a leading separator on the
// child does not make it absolute.
rooted = createObject("java", "java.io.File").init(base, "/backup.sql");
assert("a leading separator on the child stays relative to the parent",
	rooted.getPath(), twoArg.getPath());

suiteEnd();
</cfscript>
