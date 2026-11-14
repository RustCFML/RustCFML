<cfscript>
// MySQL refuses CREATE PROCEDURE / FUNCTION / TRIGGER / EVENT over the
// prepared-statement protocol (ER_UNSUPPORTED_PS, 1295). With no bind
// parameters they run as plain text, as JDBC runs a Statement.
// Live test, gated on RUSTCFML_TEST_MYSQL_DS (mysql://user:pass@host:port/db).
suiteBegin("MySQL CREATE PROCEDURE / FUNCTION through queryExecute (skipped without RUSTCFML_TEST_MYSQL_DS)");

routineDsn = "";
try {
    routineDsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    routineDsn = "";
}

if (routineDsn == "") {
    assertTrue("skipped without a MySQL test datasource", true);
} else {
    routineSuffix = lCase(left(replace(createUUID(), "-", "", "all"), 8));
    routineProc = "rcfml_add_" & routineSuffix;
    routineFn = "rcfml_twice_" & routineSuffix;
    try {
        queryExecute("CREATE PROCEDURE #routineProc#(IN a INT, IN b INT) BEGIN SELECT a + b AS total; END", [], { datasource: routineDsn });
        assert("a created procedure runs", queryExecute("CALL #routineProc#(?, ?)", [2, 3], { datasource: routineDsn }).total, 5);
        queryExecute("CREATE FUNCTION #routineFn#(n INT) RETURNS INT DETERMINISTIC RETURN n * 2", [], { datasource: routineDsn });
        assert("a created function runs", queryExecute("SELECT #routineFn#(21) AS v", [], { datasource: routineDsn }).v, 42);
    } finally {
        try { queryExecute("DROP PROCEDURE IF EXISTS #routineProc#", [], { datasource: routineDsn }); } catch (any e) {}
        try { queryExecute("DROP FUNCTION IF EXISTS #routineFn#", [], { datasource: routineDsn }); } catch (any e) {}
    }
}

suiteEnd();
</cfscript>
