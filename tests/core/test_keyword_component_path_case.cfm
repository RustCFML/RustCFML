<cfscript>
suiteBegin("A keyword-named path segment keeps its source case (GH 381)");

// A keyword token carries no text, so the parser rebuilt any keyword used as a
// name from its canonical LOWERCASE spelling. Harmless for a struct key (CFML
// keys are case-insensitive) but not for a component path, where the segment
// becomes a FILENAME: `new wheels.Public()` probed `wheels/public.cfc` and
// failed on any case-sensitive filesystem, while the equivalent
// createObject("component", "wheels.Public") — a plain string, never lexed —
// resolved fine. The divergence between the two is the tell.

assert("new resolves a camel-cased keyword-named component",
	new core.Public().whoAmI(), "core.Public");

assert("createObject resolves the same component",
	createObject("component", "core.Public").whoAmI(), "core.Public");

// The case has to survive all the way to the lookup, which is only observable
// on a case-insensitive filesystem through the error text for a path that does
// NOT resolve. `find` is case-SENSITIVE on purpose.
missing = "";
try {
	obj = new corenosuchpkg.Public();
} catch (any e) {
	missing = e.message;
}
assert("an unresolved keyword segment is reported in its source case",
	find("Public", missing) gt 0, true);
assert("it is not reported lowercased",
	find("corenosuchpkg.public", missing) gt 0, false);

// Not specific to `new`: the same reconstruction runs for every keyword used as
// a name, so a struct key written in camel case must keep it too.
s = {};
s.Public = 1;
s.Static = 2;
assert("a keyword struct key keeps its source case",
	listSort(structKeyList(s), "text"), "Public,Static");

suiteEnd();
</cfscript>
