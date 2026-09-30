<cfscript>
suiteBegin("cftransaction: every datasource a transaction touches commits and rolls back with it");

// Lucee 7.1 (probed on PostgreSQL) gives each datasource used inside a
// transaction its own connection in that transaction: a rollback, or an
// exception out of the block, undoes the writes on ALL of them, and the commit
// commits all of them. test_transaction_statement_datasource.cfm pins the
// routing; this pins the lifecycle of the second connection.

base = getTempDirectory();
stamp = getTickCount();
dbA = base & "rcfml_txmulti_a_" & stamp & ".db";
dbB = base & "rcfml_txmulti_b_" & stamp & ".db";
dsA = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbA };
dsB = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbB };

function rowsIn( required any ds ) {
	return queryExecute( "SELECT count(*) AS n FROM t", {}, { datasource: arguments.ds } ).n;
}
function clearBoth() {
	queryExecute( "DELETE FROM t", {}, { datasource: dsA } );
	queryExecute( "DELETE FROM t", {}, { datasource: dsB } );
}

try {
	queryExecute( "CREATE TABLE t (id INTEGER)", {}, { datasource: dsA } );
	queryExecute( "CREATE TABLE t (id INTEGER)", {}, { datasource: dsB } );

	transaction {
		queryExecute( "INSERT INTO t VALUES (1)", {}, { datasource: dsB } );
		queryExecute( "INSERT INTO t VALUES (1)", {}, { datasource: dsA } );
		transaction action="rollback";
	}
	assert( "rollback undoes the first datasource's write", rowsIn( dsB ), 0 );
	assert( "rollback undoes the second datasource's write", rowsIn( dsA ), 0 );

	try {
		transaction {
			queryExecute( "INSERT INTO t VALUES (2)", {}, { datasource: dsB } );
			queryExecute( "INSERT INTO t VALUES (2)", {}, { datasource: dsA } );
			throw( message = "boom" );
		}
	} catch ( any e ) {}
	assert( "an exception rolls back the first datasource", rowsIn( dsB ), 0 );
	assert( "an exception rolls back the second datasource", rowsIn( dsA ), 0 );

	transaction {
		queryExecute( "INSERT INTO t VALUES (3)", {}, { datasource: dsB } );
		queryExecute( "INSERT INTO t VALUES (3)", {}, { datasource: dsA } );
		assert( "the second datasource sees its own uncommitted write", rowsIn( dsA ), 1 );
	}
	assert( "commit keeps the first datasource's write", rowsIn( dsB ), 1 );
	assert( "commit keeps the second datasource's write", rowsIn( dsA ), 1 );

	// A datasource that first joins INSIDE a nested block: rolling the nested
	// block back must reach it too, and the outer commit keeps the rest.
	clearBoth();
	transaction {
		queryExecute( "INSERT INTO t VALUES (4)", {}, { datasource: dsB } );
		transaction {
			queryExecute( "INSERT INTO t VALUES (5)", {}, { datasource: dsA } );
			transaction action="rollback";
		}
		queryExecute( "INSERT INTO t VALUES (6)", {}, { datasource: dsA } );
	}
	assert( "nested rollback reaches a datasource that joined inside it",
		valueList( queryExecute( "SELECT id FROM t ORDER BY id", {}, { datasource: dsA } ).id ), "6" );
	assert( "the outer commit keeps the first datasource's write", rowsIn( dsB ), 1 );

	// After the transaction, both datasources are back to autocommit.
	queryExecute( "INSERT INTO t VALUES (7)", {}, { datasource: dsA } );
	assert( "the second datasource is not left inside a transaction", rowsIn( dsA ), 2 );
} finally {
	try { if ( fileExists( dbA ) ) { fileDelete( dbA ); } } catch ( any ignore ) {}
	try { if ( fileExists( dbB ) ) { fileDelete( dbB ); } } catch ( any ignore ) {}
}

suiteEnd();
</cfscript>
