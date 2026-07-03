<cfscript>
// getComponentMetaData() on a LEADING-DOT dotted path.
//
// Preside's TaskManagerConfigurationWrapper.getConfiguredTasks() builds a
// component path with `Replace( filePath, "/", ".", "all" )` where filePath is a
// leading-slash mapping path ("/preside/system/handlers/Tasks.cfc"), yielding a
// leading-dot dotted path (".preside.system.handlers.Tasks"). It then calls
// getComponentMetaData() on it and iterates `meta.functions`. Lucee ignores the
// leading dot and resolves the remainder as an ordinary dotted path; RustCFML
// used to leave the dot in place, desync every lookup, return an empty struct,
// and throw "Variable 'functions' is undefined" on the iteration. This test
// pins the Lucee-parity normalization. Verified against Lucee 7.
suiteBegin("getComponentMetaData leading-dot dotted path");

dotless = getComponentMetaData( "core.GcmLeadingDotTarget" );
leading = getComponentMetaData( ".core.GcmLeadingDotTarget" );

// The leading-dot form must resolve to the SAME component (not an empty struct).
assert("leading-dot name matches dotless", leading.name, dotless.name);
assertTrue("leading-dot has functions key", structKeyExists(leading, "functions"));
assert("leading-dot function count matches dotless",
	arrayLen(leading.functions), arrayLen(dotless.functions));

// The @schedule annotation on scheduledThing() survives (Preside reads f.schedule).
scheduled = "";
for (f in leading.functions) {
	if (f.name == "scheduledThing") {
		scheduled = f.schedule ?: "";
	}
}
assert("leading-dot preserves @schedule annotation", scheduled, "0 0 * * *");

suiteEnd();
</cfscript>
