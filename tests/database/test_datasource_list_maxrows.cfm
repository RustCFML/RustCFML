<!---
  GitHub #251: on the DATASOURCE path (not just QoQ), a list=true named param
  was bound as ONE comma-joined literal instead of expanding to one bind marker
  per element, so `WHERE id IN (:ids)` matched 0 real rows (and the equivalent
  DELETE removed nothing). The `maxrows` option was likewise ignored on the
  datasource path. The QoQ path fixed both in v0.325.0; this covers the SQLite
  datasource executor counterpart.

  Uses an on-disk-backed in-memory SQLite DS (the class/connectionString shape
  Wheels' channel DatabaseAdapter uses); DELETE-before-seed keeps runs
  deterministic because that DS persists across CLI invocations.
--->
<cfscript>
suiteBegin("datasource list:true + maxrows (GitHub 251)");

ds = {class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:"};

queryExecute("CREATE TABLE IF NOT EXISTS probe_251 (id VARCHAR(64))", {}, {datasource: ds});
queryExecute("DELETE FROM probe_251", {}, {datasource: ds});
for (i = 1; i <= 2; i++) {
    queryExecute("INSERT INTO probe_251 (id) VALUES (:id)",
        {id: {value: "evt-#i#", cfsqltype: "cf_sql_varchar"}}, {datasource: ds});
}
// Canary row whose id IS the comma-joined string — matches ONLY if the param is
// (wrongly) bound as a single literal.
queryExecute("INSERT INTO probe_251 (id) VALUES (:id)",
    {id: {value: "evt-1,evt-2", cfsqltype: "cf_sql_varchar"}}, {datasource: ds});

// (a) SELECT ... IN (:ids) with list:true expands to IN (?,?) → matches evt-1,evt-2.
sel = queryExecute("SELECT id FROM probe_251 WHERE id IN (:ids)",
    {ids: {value: "evt-1,evt-2", cfsqltype: "cf_sql_varchar", list: true}}, {datasource: ds});
assert("list:true SELECT matches both real rows (not the canary)", sel.recordCount, 2);

// (b) DELETE ... IN (:ids) with list:true removes exactly the 2 real rows,
//     leaving only the canary.
queryExecute("DELETE FROM probe_251 WHERE id IN (:ids)",
    {ids: {value: "evt-1,evt-2", cfsqltype: "cf_sql_varchar", list: true}}, {datasource: ds});
remaining = queryExecute("SELECT COUNT(*) AS c FROM probe_251", {}, {datasource: ds});
assert("list:true DELETE removes the two real rows (canary survives)", remaining.c, 1);

// (c) maxrows caps the datasource result set.
queryExecute("DELETE FROM probe_251", {}, {datasource: ds});
for (i = 1; i <= 5; i++) {
    queryExecute("INSERT INTO probe_251 (id) VALUES (:id)",
        {id: {value: "row-#i#", cfsqltype: "cf_sql_varchar"}}, {datasource: ds});
}
capped = queryExecute("SELECT id FROM probe_251 ORDER BY id", {}, {datasource: ds, maxrows: 2});
assert("maxrows caps the datasource result to 2 rows", capped.recordCount, 2);
// maxrows larger than the set returns everything.
allRows = queryExecute("SELECT id FROM probe_251 ORDER BY id", {}, {datasource: ds, maxrows: 99});
assert("maxrows above rowcount returns all rows", allRows.recordCount, 5);

queryExecute("DROP TABLE IF EXISTS probe_251", {}, {datasource: ds});

suiteEnd();
</cfscript>
