<cfscript>
// GitHub #284 (v0.511.0 regression): the first `include` of a .cfm path during a
// request must NOT pin its compiled unit for the rest of that request when the
// rustcfml process itself rewrites the file. A second include of the same path —
// here from inside a CFC method — has to re-evaluate the new content.
//
// v0.511.0 added a request-scoped freshness memo (`request_validated_files`) that
// skips the per-load mtime stat once a file was validated this request; that made
// a re-include serve the STALE compiled template even after FileWrite overwrote
// it (and even seconds later). The fix flushes that memo + the shared bytecode
// cache entry for any template THIS process writes/removes, so its own writes are
// always picked up while an external mid-request change still defers to the next
// request. See BytecodeCache::invalidate / invalidate_written_file_caches.
//
// NB: this only bites in SERVE mode (the CLI has no persistent bytecode cache, so
// each include recompiles). Under `cargo run -- tests/runner.cfm` it is a no-op
// guard; served (the release gate serves the runner cold+warm) it fails pre-fix.
suiteBegin("Include re-read after process rewrite (GitHub 284)");

// Absolute helper path so the include cache key and the FileWrite path are the
// same spelling regardless of webroot/symlinks; temp dir keeps the repo clean.
helperAbs = GetTempDirectory() & "rustcfml-gh284-" & CreateUUID() & ".cfm";

try {
	// Fresh loader instance per call (as in the issue repro): the include runs
	// against a clean component `variables` scope each time, so the collected
	// function is always the one the include just defined.
	FileWrite(helperAbs, "<cfscript>function fxProbe(){ return 'first'; }</cfscript>");
	fns1 = new oop.Gh284IncludeLoader().loadFunctions(helperAbs);
	assert("first include sees original content", fns1.fxProbe(), "first");

	// No sleep: rewriting sub-second must still be picked up (the regression was
	// time-INsensitive — even a 1500ms gap did not help pre-fix).
	FileWrite(helperAbs, "<cfscript>function fxProbe(){ return 'second'; }</cfscript>");
	fns2 = new oop.Gh284IncludeLoader().loadFunctions(helperAbs);
	assert("re-include after process rewrite sees NEW content", fns2.fxProbe(), "second");

	// And a third rewrite, to be sure it is not a one-shot invalidation.
	FileWrite(helperAbs, "<cfscript>function fxProbe(){ return 'third'; }</cfscript>");
	fns3 = new oop.Gh284IncludeLoader().loadFunctions(helperAbs);
	assert("second re-include after another rewrite sees newest content", fns3.fxProbe(), "third");
} catch (any e) {
	rethrow;
} finally {
	if (FileExists(helperAbs)) {
		FileDelete(helperAbs);
	}
}

suiteEnd();
</cfscript>
