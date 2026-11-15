<cfscript>
suiteBegin("cftransaction: nested blocks, in-block actions, savepoints and raw COMMIT/ROLLBACK (Lucee 7.1 semantics)");

// Every expected value below was produced by Lucee 7.1.2.191 against
// PostgreSQL 18, MySQL 8.4 and SQL Server (Azure SQL Edge), 2026-11-15, with
// identical results on all three except where a case says otherwise:
//
// - A nested `transaction { }` JOINS the outer transaction. It takes no
//   savepoint, and an exception caught inside the outer block rolls nothing
//   back (RustCFML used a savepoint per nested block until v0.751).
// - `action="commit"` / `"rollback"` act on the WHOLE transaction at any depth,
//   and the transaction stays open: later statements run in a new database
//   transaction that the block's end commits or rolls back (GH #537).
// - A raw COMMIT/ROLLBACK sent through queryExecute ends the database
//   transaction; the next statement begins a new one (GH #536, Preside's
//   PostgreSQL adapter). On MySQL, DDL commits implicitly the same way.
// - `action="setsavepoint"` / `rollback savepoint=` and the transaction* BIFs
//   work (GH #534); outside a transaction they do nothing.
//
// SQLite runs every time; set RUSTCFML_TEST_PG_URL, RUSTCFML_TEST_MYSQL_DS or
// RUSTCFML_TEST_MSSQL_DS to run the same cases against a live server.

function txnEnv( required string name ) {
	try {
		return server.system.environment[ arguments.name ] ?: "";
	} catch ( any e ) {
		return "";
	}
}

targets = [ { name: "sqlite", dialect: "sqlite", ds: { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & getTempDirectory() & "rcfml_txsem_" & getTickCount() & ".db" } } ];
if ( len( txnEnv( "RUSTCFML_TEST_PG_URL" ) ) ) arrayAppend( targets, { name: "postgres", dialect: "postgres", ds: txnEnv( "RUSTCFML_TEST_PG_URL" ) } );
if ( len( txnEnv( "RUSTCFML_TEST_MYSQL_DS" ) ) ) arrayAppend( targets, { name: "mysql", dialect: "mysql", ds: txnEnv( "RUSTCFML_TEST_MYSQL_DS" ) } );
if ( len( txnEnv( "RUSTCFML_TEST_MSSQL_DS" ) ) ) arrayAppend( targets, { name: "mssql", dialect: "mssql", ds: txnEnv( "RUSTCFML_TEST_MSSQL_DS" ) } );

txnTable = "zz_rcfml_txsem_" & lCase( left( replace( createUUID(), "-", "", "all" ), 8 ) );

function ins( required numeric id ) {
	queryExecute( "insert into #txnTable# values (#arguments.id#)", [], { datasource: txnDs } );
}
function raw( required string sql ) {
	queryExecute( arguments.sql, [], { datasource: txnDs } );
}
function ids() {
	return "[" & valueList( queryExecute( "select id from #txnTable# order by id", [], { datasource: txnDs } ).id ) & "]";
}
function p15ReturnFromBlock() {
	transaction { ins( 7 ); transaction action="rollback"; ins( 8 ); return; }
}

// name -> [ body, expected ]. `body` returns any extra text to compare.
cases = [
	[ "A raw COMMIT then a nested block", function() { transaction { ins( 1 ); raw( "commit" ); transaction { ins( 2 ); } } }, "[1,2]" ],
	[ "B raw COMMIT, insert, in-block rollback", function() { transaction { ins( 1 ); raw( "commit" ); ins( 2 ); transaction action="rollback"; } }, "[1]" ],
	[ "C raw ROLLBACK then insert", function() { transaction { ins( 1 ); raw( "rollback" ); ins( 2 ); } }, "[2]" ],
	[ "D raw COMMIT then two nested levels", function() { transaction { ins( 1 ); raw( "commit" ); transaction { ins( 2 ); transaction { ins( 3 ); } } } }, "[1,2,3]" ],
	[ "E in-block rollback then insert", function() { transaction { ins( 1 ); transaction action="rollback"; ins( 2 ); } }, "[2]" ],
	[ "E2 the transaction stays open after an in-block rollback", function() { transaction { ins( 1 ); transaction action="rollback"; ins( 2 ); transaction action="rollback"; } }, "[]" ],
	[ "E3 a nested block after an in-block rollback", function() { transaction { ins( 1 ); transaction action="rollback"; transaction { ins( 2 ); } ins( 3 ); } }, "[2,3]" ],
	[ "E4 in-block commit, insert, exception", function() { try { transaction { ins( 1 ); transaction action="commit"; ins( 2 ); throw( message = "boom" ); } } catch ( any e ) { return "caught "; } }, "caught [1]" ],
	[ "E5 in-block rollback, insert, exception", function() { try { transaction { ins( 1 ); transaction action="rollback"; ins( 2 ); throw( message = "boom" ); } } catch ( any e ) { return "caught "; } }, "caught []" ],
	[ "F brace rollback with a body", function() { transaction { ins( 1 ); transaction action="rollback" { ins( 2 ); } ins( 3 ); } }, "[2,3]" ],
	[ "F2 brace commit with a body", function() { transaction { ins( 1 ); transaction action="commit" { ins( 2 ); } ins( 3 ); transaction action="rollback"; } }, "[1]" ],
	[ "G dynamic action", function() { var act = "rollback"; transaction { ins( 1 ); transaction action=act; ins( 2 ); } }, "[2]" ],
	[ "H savepoint and rollback to it", function() { transaction { ins( 1 ); transaction action="setsavepoint" savepoint="sp1"; ins( 2 ); transaction action="rollback" savepoint="sp1"; ins( 3 ); } }, "[1,3]" ],
	[ "I savepoint BIFs", function() { transaction { ins( 1 ); transactionSetSavepoint( "sp1" ); ins( 2 ); transactionRollback( "sp1" ); ins( 3 ); } }, "[1,3]" ],
	[ "I2 commit and rollback BIFs", function() { transaction { ins( 1 ); transactionCommit(); ins( 2 ); transactionRollback(); } }, "[1]" ],
	[ "J actions and BIFs outside a transaction do nothing", function() { transaction action="setsavepoint" savepoint="sp1"; transaction action="rollback"; transactionRollback( "zz" ); transactionSetSavepoint( "zz" ); transactionCommit(); return "ok "; }, "ok []" ],
	[ "K blocks after an in-block rollback", function() { transaction { ins( 1 ); transaction action="rollback"; } transaction { ins( 2 ); transaction { ins( 3 ); transaction action="rollback"; } } transaction { ins( 4 ); } }, "[4]" ],
	[ "L rollback to an unknown savepoint", function() { try { transaction { ins( 1 ); transaction action="rollback" savepoint="nope"; } } catch ( any e ) { return e.message & " "; } }, "There are no savepoint with name [nope] set []" ],
	[ "M in-block rollback inside a nested block", function() { transaction { ins( 1 ); transaction { ins( 2 ); transaction action="rollback"; ins( 3 ); } ins( 4 ); } }, "[3,4]" ],
	[ "N raw COMMIT inside a nested block, outer rollback", function() { transaction { ins( 1 ); transaction { ins( 2 ); raw( "commit" ); } ins( 3 ); transaction action="rollback"; } }, "[1,2]" ],
	[ "P1 an exception caught inside the outer block rolls nothing back", function() { transaction { ins( 1 ); try { transaction { ins( 2 ); throw( message = "inner" ); } } catch ( any e ) {} ins( 3 ); } }, "[1,2,3]" ],
	[ "P2 in-block commit inside a nested block", function() { transaction { ins( 1 ); transaction { ins( 2 ); transaction action="commit"; ins( 3 ); } ins( 4 ); transaction action="rollback"; } }, "[1,2]" ],
	[ "P3 savepoint inside a nested block", function() { transaction { ins( 1 ); transaction { ins( 2 ); transaction action="setsavepoint" savepoint="a"; ins( 3 ); transaction action="rollback" savepoint="a"; ins( 4 ); } ins( 5 ); } }, "[1,2,4,5]" ],
	[ "P6 tag rollback inside a nested tag block", function() { include "txn_semantics_tag_case.cfm"; }, "[3,4]" ],
	[ "P8 unnamed savepoint; savepoint="""" rolls back everything", function() { transaction { ins( 1 ); transaction action="setsavepoint"; ins( 2 ); transaction action="rollback" savepoint=""; ins( 3 ); } }, "[3]" ],
	[ "P9 savepoint set in a nested block outlives it", function() { transaction { ins( 1 ); transaction { ins( 2 ); transaction action="setsavepoint" savepoint="a"; ins( 3 ); } ins( 4 ); transaction action="rollback" savepoint="a"; ins( 5 ); } }, "[1,2,5]" ],
	[ "P12 isWithinTransaction", function() { var o = isWithinTransaction() & " "; transaction { o &= isWithinTransaction() & " "; ins( 1 ); transaction action="rollback"; o &= isWithinTransaction() & " "; transaction { o &= isWithinTransaction() & " "; } } return o & isWithinTransaction() & " "; }, "false true true true false []" ],
	[ "P13 an exception escaping a nested block rolls back everything", function() { try { transaction { ins( 1 ); transaction { ins( 2 ); throw( message = "inner" ); } } } catch ( any e ) { return "caught "; } }, "caught []" ],
	[ "P15 return from a block after an in-block rollback commits the rest", function() { p15ReturnFromBlock(); transaction { ins( 9 ); } }, "[8,9]" ],
	[ "P16 raw ROLLBACK, then a nested block with an in-block rollback", function() { transaction { ins( 1 ); raw( "rollback" ); transaction { ins( 2 ); transaction action="rollback"; ins( 3 ); } ins( 4 ); } }, "[3,4]" ],
	[ "Q1 top-level action with a body runs the body as a transaction", function() { transaction action="rollback" { ins( 1 ); } return isWithinTransaction() & " "; }, "false [1]" ],
	[ "Q2 top-level action with a body rolls back on an exception", function() { try { transaction action="commit" { ins( 1 ); throw( message = "x" ); } } catch ( any e ) { return "caught "; } }, "caught []" ],
	[ "Q5 savepoint names match case-insensitively", function() { transaction { ins( 1 ); transaction action="setsavepoint" savepoint="SpA"; ins( 2 ); transaction action="rollback" savepoint="spa"; ins( 3 ); } }, "[1,3]" ],
	[ "Q6 setting a savepoint twice rolls back to the latest", function() { transaction { ins( 1 ); transaction action="setsavepoint" savepoint="a"; ins( 2 ); transaction action="setsavepoint" savepoint="a"; ins( 3 ); transaction action="rollback" savepoint="a"; ins( 4 ); } }, "[1,2,4]" ],
	[ "Q8 top-level setsavepoint with a body", function() { transaction action="setsavepoint" savepoint="a" { ins( 1 ); } }, "[1]" ],
	[ "Q9 action=begin with a body", function() { transaction action="begin" { ins( 1 ); transaction action="rollback"; } }, "[]" ],
	[ "R a bodyless begin statement is an empty block", function() { transaction action="begin"; var o = isWithinTransaction() & " "; ins( 1 ); transaction action="rollback"; return o; }, "false [1]" ]
];

// Cases whose result depends on the database.
dialectCases = {
	postgres: [
		[ "Q3 DDL is transactional", function() { transaction { ins( 1 ); raw( "create table #txnTable#_ddl ( x int )" ); ins( 2 ); transaction action="rollback"; } }, "[]" ],
		[ "Q7 raw COMMIT then raw BEGIN", function() { transaction { ins( 1 ); raw( "commit" ); raw( "begin" ); ins( 2 ); transaction action="rollback"; ins( 3 ); } }, "[1,3]" ],
		[ "Q10 END and ABORT end the transaction", function() { transaction { ins( 1 ); raw( "end" ); ins( 2 ); raw( "abort" ); ins( 3 ); transaction { ins( 4 ); } transaction action="rollback"; ins( 5 ); } }, "[1,5]" ]
	],
	mysql: [
		[ "Q3 DDL commits implicitly; the block stays transactional", function() { transaction { ins( 1 ); raw( "create table #txnTable#_ddl ( x int )" ); ins( 2 ); transaction action="rollback"; } }, "[1]" ],
		[ "Q7 raw COMMIT then raw BEGIN", function() { transaction { ins( 1 ); raw( "commit" ); raw( "begin" ); ins( 2 ); transaction action="rollback"; ins( 3 ); } }, "[1,3]" ]
	],
	mssql: [
		[ "Q3 DDL is transactional", function() { transaction { ins( 1 ); raw( "create table #txnTable#_ddl ( x int )" ); ins( 2 ); transaction action="rollback"; } }, "[]" ]
	],
	sqlite: [
		[ "Q3 DDL is transactional", function() { transaction { ins( 1 ); raw( "create table #txnTable#_ddl ( x int )" ); ins( 2 ); transaction action="rollback"; } }, "[]" ],
		[ "Q7 raw COMMIT then raw BEGIN", function() { transaction { ins( 1 ); raw( "commit" ); raw( "begin" ); ins( 2 ); transaction action="rollback"; ins( 3 ); } }, "[1,3]" ]
	]
};

for ( target in targets ) {
	txnDs = target.ds;
	try {
		raw( "create table #txnTable# ( id int )" );
	} catch ( any e ) {
		assertTrue( "#target.name#: skipped, could not create the table: " & e.message, true );
		continue;
	}
	try {
		for ( c in arrayMerge( cases, dialectCases[ target.dialect ] ) ) {
			raw( "delete from #txnTable#" );
			try { raw( "drop table #txnTable#_ddl" ); } catch ( any e ) {}
			out = "";
			try {
				out = c[ 2 ]() ?: "";
			} catch ( any e ) {
				out = "ERROR " & e.message & " ";
			}
			assert( "#target.name#: #c[ 1 ]#", out & ids(), c[ 3 ] );
			assertFalse( "#target.name#: #c[ 1 ]# leaves no transaction open", isWithinTransaction() );
		}
		// Savepoints die with the transaction; rolling back to one afterwards is
		// the database's own error, raised out of the block (which rolls back).
		raw( "delete from #txnTable#" );
		threw = false;
		try {
			transaction { ins( 1 ); transaction action="setsavepoint" savepoint="a"; ins( 2 ); transaction action="rollback"; ins( 3 ); transaction action="rollback" savepoint="a"; }
		} catch ( any e ) {
			threw = true;
		}
		assertTrue( "#target.name#: a savepoint is gone after a whole rollback", threw );
		assert( "#target.name#: ...and the block's work is rolled back", ids(), "[]" );
		// After everything above, a fresh transaction still works.
		raw( "delete from #txnTable#" );
		transaction { ins( 1 ); transaction { ins( 2 ); } }
		assert( "#target.name#: a fresh nested transaction afterwards", ids(), "[1,2]" );
	} finally {
		try { raw( "drop table #txnTable#_ddl" ); } catch ( any e ) {}
		try { raw( "drop table #txnTable#" ); } catch ( any e ) {}
	}
}

suiteEnd();
</cfscript>
