<cfscript>
// Test for parser fix: transaction action="rollback" {} and transaction action="commit" {}
// were incorrectly lowered to begin/commit blocks. This test uses SQLite (no RUSTCFML_TEST_PG_URL needed).

suiteBegin("cftransaction: action= with braces (parser fix)");

// Regression guard: run a fresh transaction to ensure no state leaked from previous tests (GH #224/#308)
ds = { type: "sqlite", database: "/tmp/rustcfml_txn_parser_test.db" };
queryExecute("DROP TABLE IF EXISTS txn_parser_test", [], {datasource:ds});
queryExecute("CREATE TABLE txn_parser_test (id INT)", [], {datasource:ds});
try {
	transaction {
		queryExecute("INSERT INTO txn_parser_test VALUES (999)", [], {datasource:ds});
		transaction {
			queryExecute("INSERT INTO txn_parser_test VALUES (998)", [], {datasource:ds});
		}
	}
	qGuard = queryExecute("SELECT COUNT(*) AS cnt FROM txn_parser_test", [], {datasource:ds});
	assert("regression guard: nested transaction works (no leaked state)", qGuard.cnt, 2);
	queryExecute("DELETE FROM txn_parser_test", [], {datasource:ds});
} catch (any e) {
	assert("regression guard failed (state leak from previous test)", "", "ERROR: " & e.message);
}

try {
	// Test 1: transaction action="rollback" {} should ROLLBACK, not commit
	queryExecute("DELETE FROM txn_parser_test", [], {datasource:ds});
	transaction {
		queryExecute("INSERT INTO txn_parser_test VALUES (1)", [], {datasource:ds});
		transaction action="rollback" {
			// empty body
		}
	}
	qCount = queryExecute("SELECT COUNT(*) AS cnt FROM txn_parser_test", [], {datasource:ds});
	assert("rollback with braces: count should be 0 (rolled back)", qCount.cnt, 0);
	
	// Regression guard after test 1: ensure no state leaked (GH #224/#308)
	queryExecute("DELETE FROM txn_parser_test", [], {datasource:ds});
	transaction {
		queryExecute("INSERT INTO txn_parser_test VALUES (901)", [], {datasource:ds});
		transaction {
			queryExecute("INSERT INTO txn_parser_test VALUES (902)", [], {datasource:ds});
		}
	}
	qGuard1 = queryExecute("SELECT COUNT(*) AS cnt FROM txn_parser_test", [], {datasource:ds});
	assert("after test 1: nested transaction works (no leaked state)", qGuard1.cnt, 2);

	// Test 2: transaction action="commit" {} should COMMIT immediately
	queryExecute("DELETE FROM txn_parser_test", [], {datasource:ds});
	transaction {
		queryExecute("INSERT INTO txn_parser_test VALUES (2)", [], {datasource:ds});
		transaction action="commit" {
			// empty body
		}
		queryExecute("INSERT INTO txn_parser_test VALUES (3)", [], {datasource:ds});
	}
	qCount = queryExecute("SELECT COUNT(*) AS cnt FROM txn_parser_test", [], {datasource:ds});
	qRows = queryExecute("SELECT id FROM txn_parser_test ORDER BY id", [], {datasource:ds});
	// Row 2 was committed, row 3 was inserted after and committed at block end
	assert("commit with braces: count should be 2", qCount.cnt, 2);
	if (qRows.recordCount >= 2) {
		assert("commit with braces: first row is 2", qRows.id[1], 2);
		assert("commit with braces: second row is 3", qRows.id[2], 3);
	}
	
	// Final regression guard: ensure all tests left no leaked state
	queryExecute("DELETE FROM txn_parser_test", [], {datasource:ds});
	transaction {
		queryExecute("INSERT INTO txn_parser_test VALUES (801)", [], {datasource:ds});
	}
	qGuard2 = queryExecute("SELECT COUNT(*) AS cnt FROM txn_parser_test", [], {datasource:ds});
	assert("after all tests: simple transaction works (no leaked state)", qGuard2.cnt, 1);

} finally {
	try {
		queryExecute("DROP TABLE txn_parser_test", [], {datasource:ds});
	} catch (any ignore) {}
}

suiteEnd();
</cfscript>
