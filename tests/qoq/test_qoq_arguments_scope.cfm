<cfscript>
// QoQ must resolve a source query held in the `arguments` scope when the source
// table is named `arguments.rsX`. Inside a CFC method the arguments scope lives
// under a reserved key (not a literal "arguments" local), so find_query_in_scope
// couldn't find it and raised "table 'arguments.rsplugins' not found". This is
// Masa's admin framework.cfc pluginConfig path (cPlugins.list). Extends the
// v0.484 component-`variables`-scope QoQ fix. Cross-engine (RustCFML + Lucee).

suiteBegin("QoQ Arguments Scope");

qoqScopeCFC = createObject("component", "qoq_scope_helper");
srcQ = queryNew("id,val", "integer,varchar", [[1, "a"], [2, "b"], [3, "c"]]);
res = qoqScopeCFC.buildFromArgs(rsplugins = srcQ);

assert("filtered recordCount (id >= 2)", res.recordCount, 2);
assert("first id", res.id[1], 2);
assert("last id", res.id[2], 3);
assert("value carried", res.val[1], "b");

suiteEnd();
</cfscript>
