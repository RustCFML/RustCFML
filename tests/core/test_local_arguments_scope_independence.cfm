<cfscript>
// A `local`-scoped variable named `arguments` is an ordinary local struct
// (Lucee parity).
//
// `local.arguments` is a variable in the `local` scope whose name happens to
// be "arguments" — it is not the `arguments` scope. Assigning and reading it
// must behave like any other local struct: the keys written to it are the
// keys it has.
//
// Real-world hit: a Moopa route dispatcher (a custom tag, no declared
// arguments) builds a working struct `local.arguments = {}`, fills it with
// the matched route params, and passes it to the endpoint. When the engine
// treats `local.arguments` as the `arguments` scope, the struct reads back
// EMPTY, the route params are lost, and every parameterised route 500s with
// "No ID provided".

suiteBegin("Core: local-scoped 'arguments' variable behaves as an ordinary struct");

o = createObject("component", "LocalArgumentsVarFixture");
r = o.build();

assertTrue("local.arguments keeps the keys written to it (got: [" & r & "])",
	find("keys=[route,track_id,extra]", r) GT 0);

assertTrue("local.arguments values are readable (got: [" & r & "])",
	find("track_id=[THE-ID]", r) GT 0);

assertTrue("structAppend into local.arguments works (got: [" & r & "])",
	find("extra=[Z]", r) GT 0);

suiteEnd();
</cfscript>
