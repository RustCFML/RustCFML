<cfscript>
// QoQ must resolve source query variables from the component `variables` scope
// when run inside a CFC method. Masa's pluginManager.loadPlugins builds
// `variables.rsScripts` via one QoQ, then a second QoQ reads it back — the
// second failed with "table 'rsScripts' not found" because find_query_in_scope
// only searched the frame locals + page globals, never the component `__variables`.
// Cross-engine (RustCFML + Lucee).

suiteBegin("QoQ Component Variables Scope");

qoqScopeCFC = createObject("component", "qoq_scope_helper");
qResult = qoqScopeCFC.build();

// Second QoQ (reads back `variables.merged`) returns the union, ordered.
assert("union recordCount", qResult.recordCount, 3);
assert("order first id", qResult.id[1], 1);
assert("order last id", qResult.id[3], 3);
assert("value carried", qResult.val[3], "z");

suiteEnd();
</cfscript>
