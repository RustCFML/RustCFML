<cfscript>
suiteBegin("Lucee Query builder (new Query())");

// Lucee ships a built-in `new Query()` programmatic query builder
// (org.lucee.cfml.Query). RustCFML provides an engine-bundled compat shim,
// overlaid onto the VFS, backed by queryExecute. Preside's
// PresideObjectServiceTest instantiates `new query()` directly to set up /
// inspect fixture data, so the engine must resolve it out of the box.

// 1. Instantiation resolves the engine-bundled component.
q = new Query();
assertTrue("new Query() resolves to a component", isObject(q));

// 2. Fluent builder + execute() returning a Result with getResult().
q = new Query();
q.setSQL("select 1 as one, 'a' as two");
result = q.execute();
assertTrue("execute() returns an object", isObject(result));

rs = result.getResult();
assertTrue("getResult() returns a query", isQuery(rs));
assert("recordCount is 1", rs.recordCount, 1);
assert("column one selected", rs.one, 1);
assert("column two selected", rs.two, "a");

// 3. getPrefix() exposes result metadata.
prefix = result.getPrefix();
assert("prefix recordcount", prefix.recordcount, 1);

// 4. Lowercase `new query()` resolves the same shim (CFML is case-insensitive).
q2 = new query();
q2.setSQL("select 42 as answer");
assert("lowercase new query() executes", q2.execute().getResult().answer, 42);

// 5. Positional params bind via addParam() (`?` placeholder).
q3 = new Query();
q3.setSQL("select ? as bound");
q3.addParam(value = 7, cfsqltype = "cf_sql_integer");
assert("positional addParam binds", q3.execute().getResult().bound, 7);

// 6. Named params bind via setParams() (`:name` placeholder, struct form).
q4 = new Query();
q4.setSQL("select :wanted as bound");
q4.setParams({ wanted = { value = 5, cfsqltype = "cf_sql_integer" } });
assert("named setParams binds", q4.execute().getResult().bound, 5);

// 7. Query-of-Queries via setDbType('query') + setAttributes(name=queryVar).
//    This is the exact idiom Masa CMS's home.cfc uses to pick a default site:
//      qs.setDbType('query');
//      qs.setAttributes(rsList=rsList);
//      qs.execute(sql="SELECT ... FROM rsList WHERE ...").getResult();
//    The in-memory query is provided as an ATTRIBUTE (not a scope variable), so
//    the QoQ engine must resolve the source table from the queryExecute options.
srcQry = queryNew("siteid,orderno", "varchar,integer",
    [ ["alpha", 2], ["bravo", 1], ["charlie", 3] ]);

qq = new Query();
qq.addParam(name = "wantid", cfsqltype = "cf_sql_varchar", value = "bravo");
qq.setDbType('query');
qq.setAttributes(rsList = srcQry);
qqRes = qq.execute(sql = "SELECT siteid FROM rsList WHERE siteid = :wantid").getResult();
assert("QoQ via setAttributes finds the attribute-provided query (recordCount)", qqRes.recordCount, 1);
assert("QoQ via setAttributes returns the right row", qqRes.siteid, "bravo");
assert("getDbType() reflects setDbType", qq.getDbType(), "query");

// 7b. QoQ with ORDER BY over the attribute-provided query (home.cfc's 2nd path).
qq2 = new Query();
qq2.setDbType('query');
qq2.setAttributes(rsList = srcQry);
ordered = qq2.execute(sql = "SELECT siteid FROM rsList ORDER BY orderno").getResult();
assert("QoQ ORDER BY row count", ordered.recordCount, 3);
assert("QoQ ORDER BY first row (orderno asc)", ordered.siteid, "bravo");

suiteEnd();
</cfscript>
