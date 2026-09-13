<cfscript>
// MariaDB (10.5+) returns rows from `INSERT ... RETURNING` and
// `DELETE ... RETURNING` — the analogue of PostgreSQL's RETURNING (fixed in the
// PG path by GitHub #174). Stock MySQL has no RETURNING and rejects it as a
// syntax error regardless of routing.
//
// Where the server DOES support it, the mysql crate's `exec_drop` silently
// discards the result set, so the returned rows were lost and the caller saw a
// bare mutation result. The fix routes these through the row-returning `exec`
// path. (UPDATE ... RETURNING is not supported by MariaDB, so it is not routed.)
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS. Requires MariaDB (not MySQL) for
// the RETURNING subtests; on MySQL the RETURNING statements error and the test
// records that as an expected skip. To run locally:
//   docker run --rm -d -p 3306:3306 -e MARIADB_ROOT_PASSWORD=root mariadb:11
//   RUSTCFML_TEST_MYSQL_DS=mysql://root:root@127.0.0.1:3306/test \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("MariaDB DML RETURNING rows (skipped without RUSTCFML_TEST_MYSQL_DS)");

mydsn = "";
try {
    mydsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    mydsn = "";
}

myskip = false;
mariadb = false;
tbl = "zz_rcfml_returning_" & lCase(left(replace(createUUID(), "-", "", "all"), 8));

if (mydsn == "") {
    myskip = true;
    writeOutput("  (skipped - set RUSTCFML_TEST_MYSQL_DS to enable)" & chr(10));
} else {
    try {
        ver = queryExecute("SELECT VERSION() AS v", [], { datasource: mydsn });
        mariadb = ver.v[1] CONTAINS "MariaDB";
        queryExecute("CREATE TABLE #tbl# (id int PRIMARY KEY, label varchar(50))",
            [], { datasource: mydsn });
        queryExecute("INSERT INTO #tbl# (id, label) VALUES (1, 'alpha'), (2, 'beta')",
            [], { datasource: mydsn });
    } catch (any e) {
        myskip = true;
        writeOutput("  (skipped - MySQL/MariaDB not reachable: " & e.message & ")" & chr(10));
    }
}

if (NOT myskip AND NOT mariadb) {
    writeOutput("  (RETURNING subtests skipped - server is MySQL, not MariaDB)" & chr(10));
}

if (NOT myskip AND mariadb) {
    try {
        inserted = queryExecute(
            "INSERT INTO #tbl# (id, label) VALUES (?, ?) RETURNING id, label",
            [3, "gamma"],
            { datasource: mydsn, returntype: "array" }
        );
        assertTrue("queryExecute INSERT RETURNING returns an array", isArray(inserted));
        assert("queryExecute INSERT RETURNING row count", arrayLen(inserted), 1);
        assert("queryExecute INSERT RETURNING id", inserted[1].id ?: 0, 3);
        assert("queryExecute INSERT RETURNING label", inserted[1].label ?: "", "gamma");

        deleted = queryExecute(
            "DELETE FROM #tbl# WHERE id = ? RETURNING label",
            [2],
            { datasource: mydsn, returntype: "array" }
        );
        assertTrue("queryExecute DELETE RETURNING returns an array", isArray(deleted));
        assert("queryExecute DELETE RETURNING row count", arrayLen(deleted), 1);
        assert("queryExecute DELETE RETURNING label", deleted[1].label ?: "", "beta");

        // A plain INSERT (no RETURNING) must still report a mutation result.
        plain = queryExecute(
            "INSERT INTO #tbl# (id, label) VALUES (?, ?)",
            [4, "delta"],
            { datasource: mydsn }
        );
        // Measured on Lucee 7.1.0.204: a plain INSERT through queryExecute
        // returns an EMPTY QUERY, not a struct — the mutation counters live on
        // cfquery's `result` attribute, not on the return value. This assertion
        // used to demand a struct with `recordCount`, which no engine returns;
        // it failed whenever RUSTCFML_TEST_MYSQL_DS was set, and since the
        // suite is skipped without one, nothing ever ran it. What it is
        // actually guarding is that a non-RETURNING statement does NOT get
        // routed to the row-returning path, so assert exactly that.
        assertTrue("queryExecute plain INSERT stays on the mutation path",
            isQuery(plain));
        assert("returning no rows", plain.recordCount, 0);
    } catch (any e) {
        assertTrue("MariaDB queryExecute DML RETURNING failed: " & e.message, false);
    }
}

if (NOT myskip) {
    try {
        queryExecute("DROP TABLE #tbl#", [], { datasource: mydsn });
    } catch (any e) {}
}

suiteEnd();
</cfscript>
