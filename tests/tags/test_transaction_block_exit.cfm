<cfscript>
suiteBegin("cftransaction: the block ends however control leaves it (GH ##308)");

// ============================================================
// Background (GH #308, reported against v0.547.0)
// ============================================================
// `transaction { ... }` lowered to
//     __cftransaction_start(); try { body; commit(); } catch { rollback(); throw; }
// with NO `finally`. So control leaving the body any other way — a `return`, or
// a `break` out of an enclosing loop — ran NEITHER the commit nor the rollback:
//
//   * the work was silently never committed, and
//   * `transaction_depth` stayed raised with the connection still open, so every
//     later `transaction { }` in the same request became a nested SAVEPOINT on
//     a stale connection instead of a real transaction.
//
// The reported symptom was the second effect leaking across TestBox bundles in
// one request: running Preside's TaskManagerServiceTest first made
// LoginServiceTest die at setup with
// "savepoint error: SAVEPOINT cftxn_sp2 does not exist" — MySQL's BEGIN on the
// reused dirty connection implicitly commits and drops every savepoint.
//
// Lucee 7.0.4 (probed on HSQLDB, same script) commits on both the `return` and
// the `break` exit and rolls back on the throw — the expectations below.
//
// HOW THE ASSERTIONS DETECT AN OPEN BLOCK: while a transaction is open every
// query in the request is routed through its connection, so simply counting
// rows cannot tell committed from uncommitted. Instead each shape is followed
// by a bare `transaction action="rollback";`. If the block was left OPEN that
// stray rollback is the block's own top-level rollback and its row vanishes; if
// the block was properly ended the row is already durable and the rollback has
// nothing to undo.
//
// Driven on SQLite so no server is needed. Lucee ships no SQLite driver, so the
// suite skips there with one informational pass rather than spraying reds.
// ============================================================

txnDbFile = getTempDirectory() & "/rustcfml_txn308_" & createUUID() & ".db";
txnDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & txnDbFile };

function txnIns( ds, id ) { queryExecute( "insert into txnexit (id) values (#id#)", {}, { datasource = ds } ); }
function txnRows( ds ) { return valueList( queryExecute( "select id from txnexit order by id", {}, { datasource = ds } ).id ); }
// Closes anything the shape left open. A no-op once the block ends properly.
function txnStrayRollback() { try { transaction action="rollback"; } catch ( any e ) {} }

function txnShapeReturn( ds ) { transaction { txnIns( ds, 1 ); return; } }
function txnShapeBreak( ds ) { for ( i = 1; i <= 3; i++ ) { transaction { txnIns( ds, 2 ); break; } } }
function txnShapeNestedReturn( ds ) {
	transaction { txnIns( ds, 3 ); transaction { txnIns( ds, 4 ); return; } }
}
function txnShapeThreadReturn( ds ) {
	thread name="txn308thread" ds="#ds#" {
		transaction { txnIns( attributes.ds, 5 ); return; }
	}
	threadJoin( "txn308thread", 10000 );
}

txnDriver = true;
try {
	queryExecute( "create table txnexit ( id int primary key )", {}, { datasource = txnDs } );
} catch ( any e ) {
	txnDriver = false;
}

if ( !txnDriver ) {
	assertTrue( "skipped — no SQLite JDBC driver on this engine", true );
} else {
	// --- a `return` out of the block commits it ---
	txnShapeReturn( txnDs );
	txnStrayRollback();
	assert( "a return out of transaction{} commits the block", txnRows( txnDs ), "1" );

	// --- a `break` out of the block commits it ---
	txnShapeBreak( txnDs );
	txnStrayRollback();
	assert( "a break out of transaction{} commits the block", txnRows( txnDs ), "1,2" );

	// --- a return out of a NESTED block ends both levels ---
	txnShapeNestedReturn( txnDs );
	txnStrayRollback();
	assert( "a return out of a nested transaction{} commits both levels", txnRows( txnDs ), "1,2,3,4" );

	// --- the same shape inside a cfthread ---
	txnShapeThreadReturn( txnDs );
	txnStrayRollback();
	assert( "a return out of transaction{} inside cfthread commits the block", txnRows( txnDs ), "1,2,3,4,5" );

	// --- CONTROL: an exception still rolls the block back ---
	try {
		transaction {
			txnIns( txnDs, 6 );
			throw( message = "boom" );
		}
	} catch ( any e ) {}
	assert( "CONTROL: a throw out of transaction{} still rolls it back", txnRows( txnDs ), "1,2,3,4,5" );

	// --- CONTROL: a normal block still commits ---
	transaction {
		txnIns( txnDs, 7 );
	}
	txnStrayRollback();
	assert( "CONTROL: a block that reaches its closing brace commits", txnRows( txnDs ), "1,2,3,4,5,7" );

	try { fileDelete( txnDbFile ); } catch ( any e ) {}
}

suiteEnd();
</cfscript>
