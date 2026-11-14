<cfscript>
// SQL Server batches that set up and then SELECT return the SELECT's rows, as
// on Lucee. They used to take the mutation path and return nothing.
// Live test, gated on RUSTCFML_TEST_MSSQL_DS (see test_mssql_tls.cfm).
suiteBegin("SQL Server setup-then-SELECT batches (skipped without RUSTCFML_TEST_MSSQL_DS)");

batchDsn = "";
try {
    batchDsn = server.system.environment.RUSTCFML_TEST_MSSQL_DS ?: "";
} catch (any e) {
    batchDsn = "";
}

if (!len(batchDsn)) {
    assertTrue("skipped: RUSTCFML_TEST_MSSQL_DS not set", true);
} else {
    r = queryExecute("DECLARE @n INT = 41; SELECT @n + 1 AS answer", [], { datasource: batchDsn });
    assert("DECLARE then SELECT returns the row", r.answer, 42);
    r = queryExecute("SET NOCOUNT ON; SELECT 'x' AS v", [], { datasource: batchDsn });
    assert("SET NOCOUNT ON then SELECT returns the row", r.v, "x");
    r = queryExecute("IF 1 = 1 SELECT 3 AS three", [], { datasource: batchDsn });
    assert("IF ... SELECT returns the row", r.three, 3);
    r = queryExecute("DECLARE @t TABLE (id int); INSERT INTO @t VALUES (7), (8); SELECT id FROM @t ORDER BY id", [], { datasource: batchDsn });
    assert("a table variable filled then selected", valueList(r.id), "7,8");
}

suiteEnd();
</cfscript>
