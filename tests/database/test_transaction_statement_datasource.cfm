<cfscript>
suiteBegin("cftransaction: a statement runs on the datasource it names");

// Inside a transaction, every statement currently runs on the datasource the
// transaction used FIRST, whatever datasource the statement names. So a write
// meant for A silently lands in B. Outside a transaction the same two statements
// route correctly. Lucee and BoxLang run each statement on its own datasource;
// Adobe ColdFusion rejects the mix ("Datasource names for all the database tags
// within the cftransaction tag must be the same"). Either is acceptable here;
// running the statement against the wrong database is not.

base = getTempDirectory();
stamp = getTickCount();
dbA = base & "rcfml_txds_a_" & stamp & ".db";
dbB = base & "rcfml_txds_b_" & stamp & ".db";
dsA = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbA };
dsB = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbB };

function tableIn( required any ds, required string name ) {
	return queryExecute(
		"SELECT count(*) AS n FROM sqlite_master WHERE type = 'table' AND name = :name",
		{ name: arguments.name },
		{ datasource: arguments.ds }
	).n;
}

try {
	// Control: no transaction.
	queryExecute( "SELECT 1 AS x", {}, { datasource: dsB } );
	queryExecute( "CREATE TABLE outside_tx (id INTEGER)", {}, { datasource: dsA } );
	assert( "outside a transaction, the table is created in A", tableIn( dsA, "outside_tx" ), 1 );
	assert( "outside a transaction, the table is not created in B", tableIn( dsB, "outside_tx" ), 0 );

	// The transaction uses B first; the next statement names A.
	outcome = "ran";
	try {
		transaction {
			queryExecute( "SELECT 1 AS x", {}, { datasource: dsB } );
			queryExecute( "CREATE TABLE inside_tx (id INTEGER)", {}, { datasource: dsA } );
		}
	} catch ( database e ) {
		outcome = "rejected";
	}
	if ( outcome == "ran" ) {
		assert( "inside a transaction, the statement for A creates its table in A", tableIn( dsA, "inside_tx" ), 1 );
		assert( "inside a transaction, the statement for A does not run in B", tableIn( dsB, "inside_tx" ), 0 );
	} else {
		assert( "a rejected mix leaves B untouched", tableIn( dsB, "inside_tx" ), 0 );
	}
} finally {
	try { if ( fileExists( dbA ) ) { fileDelete( dbA ); } } catch ( any ignore ) {}
	try { if ( fileExists( dbB ) ) { fileDelete( dbB ); } } catch ( any ignore ) {}
}

suiteEnd();
</cfscript>
