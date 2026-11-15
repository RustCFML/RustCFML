<cfscript>
suiteBegin("cftransaction: raw COMMIT then nested transaction (PostgreSQL)");

// Background: Preside's PostgreSQL adapter runs a raw `COMMIT` SQL statement
// after inserts (requiresManualCommitForTransactions()=true). When that happens
// inside a `transaction {}` block, it ends the transaction RustCFML opened with
// BEGIN. A nested `transaction {}` then issues SAVEPOINT, and PostgreSQL fails:
// "SAVEPOINT can only be used in transaction blocks". Lucee with pgjdbc (autocommit
// off) lazily re-issues BEGIN before the next statement, so this works.
//
// This test verifies RustCFML matches that behavior: after a raw COMMIT or ROLLBACK,
// a nested transaction (SAVEPOINT) should trigger a lazy BEGIN.

dsname = server.system.environment.RUSTCFML_TEST_PG_URL ?: "";
if (!len(dsname)) {
	writeOutput("  (skipped — set RUSTCFML_TEST_PG_URL to enable)" & chr(10));
	suiteEnd();
	return;
}

ds = { type: "postgres", connectionString: dsname };

// Setup
queryExecute("DROP TABLE IF EXISTS rcfml_tx_probe", [], {datasource:ds});
queryExecute("CREATE TABLE rcfml_tx_probe ( id INT )", [], {datasource:ds});

try {
	// Test A: bare commit then nested transaction
	queryExecute("DELETE FROM rcfml_tx_probe", [], {datasource:ds});
	try {
		transaction {
			queryExecute("INSERT INTO rcfml_tx_probe VALUES (1)", [], {datasource:ds});
			queryExecute("COMMIT", [], {datasource:ds});
			transaction {
				queryExecute("INSERT INTO rcfml_tx_probe VALUES (2)", [], {datasource:ds});
			}
		}
		var qCount = queryExecute("SELECT COUNT(*) AS cnt FROM rcfml_tx_probe", [], {datasource:ds});
		assert("A: nested transaction after COMMIT works", qCount.cnt, 2);
	} catch (any e) {
		assert("A: should not throw", "", "ERROR: " & e.message);
	}

	// Test B: bare commit, more statements, then rollback outer
	queryExecute("DELETE FROM rcfml_tx_probe", [], {datasource:ds});
	try {
		transaction {
			queryExecute("INSERT INTO rcfml_tx_probe VALUES (10)", [], {datasource:ds});
			queryExecute("COMMIT", [], {datasource:ds});
			queryExecute("INSERT INTO rcfml_tx_probe VALUES (20)", [], {datasource:ds});
			transaction action="rollback" {}
		}
	} catch (any e) {
		// ignore rollback errors
	}
	qCount = queryExecute("SELECT COUNT(*) AS cnt FROM rcfml_tx_probe", [], {datasource:ds});
	qRows = queryExecute("SELECT id FROM rcfml_tx_probe ORDER BY id", [], {datasource:ds});
	assert("B: only row 10 survives (committed before COMMIT)", qCount.cnt, 1);
	if (qRows.recordCount > 0) {
		assert("B: surviving row is ID 10", qRows.id[1], 10);
	}

	// Test C: bare rollback mid-transaction
	queryExecute("DELETE FROM rcfml_tx_probe", [], {datasource:ds});
	try {
		transaction {
			queryExecute("INSERT INTO rcfml_tx_probe VALUES (30)", [], {datasource:ds});
			queryExecute("ROLLBACK", [], {datasource:ds});
			queryExecute("INSERT INTO rcfml_tx_probe VALUES (40)", [], {datasource:ds});
		}
	} catch (any e) {
		// ignore any errors
	}
	qCount = queryExecute("SELECT COUNT(*) AS cnt FROM rcfml_tx_probe", [], {datasource:ds});
	qRows = queryExecute("SELECT id FROM rcfml_tx_probe ORDER BY id", [], {datasource:ds});
	// After ROLLBACK, the transaction is closed, so the next INSERT runs outside
	// a transaction (after lazy re-BEGIN) and commits immediately
	assert("C: only row 40 survives (inserted after ROLLBACK)", qCount.cnt, 1);
	if (qRows.recordCount > 0) {
		assert("C: surviving row is ID 40", qRows.id[1], 40);
	}

	// Test D: nested transaction with savepoint after commit
	queryExecute("DELETE FROM rcfml_tx_probe", [], {datasource:ds});
	try {
		transaction {
			queryExecute("INSERT INTO rcfml_tx_probe VALUES (100)", [], {datasource:ds});
			queryExecute("COMMIT", [], {datasource:ds});
			// This nested transaction should trigger lazy BEGIN before SAVEPOINT
			transaction {
				queryExecute("INSERT INTO rcfml_tx_probe VALUES (200)", [], {datasource:ds});
				transaction {
					queryExecute("INSERT INTO rcfml_tx_probe VALUES (300)", [], {datasource:ds});
				}
			}
		}
		qCount = queryExecute("SELECT COUNT(*) AS cnt FROM rcfml_tx_probe", [], {datasource:ds});
		assert("D: all three rows inserted", qCount.cnt, 3);
	} catch (any e) {
		assert("D: should not throw", "", "ERROR: " & e.message);
	}

} finally {
	try {
		queryExecute("DROP TABLE IF EXISTS rcfml_tx_probe", [], {datasource:ds});
	} catch (any ignore) {}
}

suiteEnd();
</cfscript>
