<cfscript>
// A `module` (or custom tag) invoked INSIDE a cfthread that carries its own
// attributes must see ITS OWN attributes, not the thread's. Reported by
// jjannek (GH ##324), measured on Lucee 5.4.8.2 and re-measured here on Lucee
// 7.0.4: the module template's `attributes` resolved to the THREAD's
// attributes scope and the module's own attributes were lost entirely, so
// reading any of them threw "Variable 'X' is undefined". A thread WITHOUT
// attributes fell through to the module's and worked, which masked the bug in
// most synthetic tests.
//
// Real-world class: Preside renders every email template through
// `module attributeCollection=…` (coldboxModifications/services/Renderer.cfc ->
// RendererEncapsulator.cfm, whose first line reads attributes.rendererVariables),
// and its adhoc background tasks always run in cfthreads carrying task-argument
// attributes — so every template render inside a background task died.
//
// The control legs matter as much as the fix: the thread's OWN attributes must
// still shadow an inherited/captured `attributes` once the module returns
// (v0.205.0 behaviour — this whole suite runs inside the runtest.cfm CUSTOM
// TAG, whose attributes the thread closure captures, so that is live here).
//
// PATH NOTE (cross-engine): inside a cfthread Lucee has no calling-template
// base, so a RELATIVE `module template=` resolves against the server context
// library and fails — it needs the webroot-absolute spelling. RustCFML resolves
// it relative to the including template and has no webroot at the CLI, so it
// needs the relative one. Each call therefore tries the relative spelling and
// falls back to the absolute one; resolution fails before any output is
// emitted, so the savecontent capture only ever holds the successful run.

suiteBegin("cfthread + module: the module's own attributes scope wins (GH ##324)");

// ── The gap: thread WITH attributes, named module attributes ──
thread name="t_attrs" someThreadAttr="hello" {
	savecontent variable="o" {
		try { module template="customtags/cfthread_module_attributes_probe.cfm" marker="module-attr"; }
		catch (any e) { module template="/tests/tags/customtags/cfthread_module_attributes_probe.cfm" marker="module-attr"; }
	}
	thread.probe = trim( o );
	// After the module returns, the THREAD's attributes must be visible again.
	thread.afterKeys = listSort( structKeyList( attributes ), "textnocase" );
}
threadJoin( "t_attrs" );

assert( "module inside a thread WITH attributes sees its OWN attributes",
	cfthread.t_attrs.probe, "keys=[marker] marker=[module-attr]" );
assert( "the thread's own attributes are visible again after the module returns",
	cfthread.t_attrs.afterKeys, "somethreadattr" );

// ── Control: thread WITHOUT attributes (worked before the fix) ──
thread name="t_noattrs" {
	savecontent variable="o2" {
		try { module template="customtags/cfthread_module_attributes_probe.cfm" marker="module-attr"; }
		catch (any e) { module template="/tests/tags/customtags/cfthread_module_attributes_probe.cfm" marker="module-attr"; }
	}
	thread.probe = trim( o2 );
}
threadJoin( "t_noattrs" );

assert( "module inside a thread WITHOUT attributes sees its own attributes",
	cfthread.t_noattrs.probe, "keys=[marker] marker=[module-attr]" );

// ── attributeCollection form (the shape Preside's Renderer uses) ──
thread name="t_collection" taskArg="bg-task" {
	savecontent variable="o3" {
		try { module template="customtags/cfthread_module_attributes_probe.cfm" attributeCollection={ marker = "module-attr", extra = "e" }; }
		catch (any e) { module template="/tests/tags/customtags/cfthread_module_attributes_probe.cfm" attributeCollection={ marker = "module-attr", extra = "e" }; }
	}
	thread.probe = trim( o3 );
}
threadJoin( "t_collection" );

assert( "attributeCollection inside a thread WITH attributes delivers the module's attributes",
	cfthread.t_collection.probe, "keys=[extra,marker] marker=[module-attr]" );

// ── A thread body that never enters a tag still reads the THREAD's attributes,
//    even though the enclosing runtest.cfm custom tag's attributes are captured
//    by the thread closure (the v0.205.0 contract this arm exists for).
thread name="t_plain" someThreadAttr="hello" {
	thread.threadKeys = listSort( structKeyList( attributes ), "textnocase" );
	thread.hasThreadAttr = structKeyExists( attributes, "someThreadAttr" );
}
threadJoin( "t_plain" );

assert( "a plain thread body still reads the THREAD's attributes, not the enclosing custom tag's",
	cfthread.t_plain.threadKeys, "somethreadattr" );
assertTrue( "the thread attribute itself is readable",
	cfthread.t_plain.hasThreadAttr );

// ── Outside any thread, a module still gets its own attributes ──
savecontent variable="topLevel" {
	try { module template="customtags/cfthread_module_attributes_probe.cfm" marker="module-attr"; }
	catch (any e) { module template="/tests/tags/customtags/cfthread_module_attributes_probe.cfm" marker="module-attr"; }
}
assert( "top level (no thread): module sees its own attributes",
	trim( topLevel ), "keys=[marker] marker=[module-attr]" );

suiteEnd();
</cfscript>
