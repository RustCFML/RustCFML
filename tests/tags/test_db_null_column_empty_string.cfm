<cfscript>
// A SQL NULL column read back from a live server adapter must surface as an
// empty string (CFML/Lucee/ACF default: full null support OFF), NOT as
// CfmlValue::Null. This mirrors the SQLite (`SqlValue::Null => ""`) and MySQL
// (`mysql::Value::NULL => ""`) adapters. GitHub #264 (MSSQL) and #265 (Postgres)
// tracked the two server adapters that still diverged; both are normalized at
// the row-converter entry point (postgres_row_to_cfml / mssql_column_to_cfml).
//
// Repro class: on MySQL this exact divergence broke Preside sitetree editPage on
// the homepage (parent_page = NULL) — a Null passed positionally left a required
// arg unbound → "Variable 'parentId' is undefined". The server adapters would
// hit the identical failure.
//
// Live tests, each gated on its DS env var (skipped when unset). To run locally:
//   docker run -d -p 55432:5432 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=testdb postgres:16
//   docker run -d -p 51433:1433 -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD=Test_pass123 \
//     mcr.microsoft.com/mssql/server:2022-latest
//   RUSTCFML_TEST_PG_DS=postgres://postgres:test@127.0.0.1:55432/testdb \
//   RUSTCFML_TEST_MSSQL_DS=mssql://sa:Test_pass123@127.0.0.1:51433/master \
//   RUSTCFML_TEST_MYSQL_DS=mysql://root:root@127.0.0.1:3306/test \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("DB NULL column reads as empty string (live; skipped without DS env vars)");

function envDs(required string varName) {
	try { return getEnvironmentVariable(arguments.varName, ""); }
	catch (any e) { return ""; }
}

// Shared assertions: a table with a NULL column + a present column, read back.
function assertNullColEmpty(required string engine, required string ds) {
	try { queryExecute("DROP TABLE rcfml_null_test", [], {datasource: arguments.ds}); } catch (any e) {}
	queryExecute(
		"CREATE TABLE rcfml_null_test (id int, nullable_col varchar(50), present_col varchar(50))",
		[], {datasource: arguments.ds}
	);
	queryExecute(
		"INSERT INTO rcfml_null_test (id, nullable_col, present_col) VALUES (1, NULL, 'hi')",
		[], {datasource: arguments.ds}
	);
	q = queryExecute("SELECT nullable_col, present_col FROM rcfml_null_test WHERE id = 1", [], {datasource: arguments.ds});

	v = q.nullable_col[1];
	assertFalse( "#engine#: NULL cell is not isNull()", isNull(v) );
	assert(      "#engine#: NULL cell reads as empty string", v, "" );
	assertTrue(  "#engine#: NULL cell len() == 0", len(v) == 0 );
	assertTrue(  "#engine#: NULL cell EQ ''", v EQ "" );
	assert(      "#engine#: non-null cell intact", q.present_col[1], "hi" );

	// The Preside repro: a NULL column passed positionally to a required arg.
	function needsRequired( required string parentId ) { return "ok:[" & arguments.parentId & "]"; }
	assert( "#engine#: NULL col binds to a required positional arg", needsRequired( q.nullable_col[1] ), "ok:[]" );

	try { queryExecute("DROP TABLE rcfml_null_test", [], {datasource: arguments.ds}); } catch (any e) {}
}

// ---- PostgreSQL (#265) ----
pgDs = envDs("RUSTCFML_TEST_PG_DS");
if ( len(pgDs) == 0 ) {
	assertTrue("PostgreSQL NULL-column check skipped (RUSTCFML_TEST_PG_DS not set)", true);
} else {
	assertNullColEmpty("PostgreSQL", pgDs);
}

// ---- SQL Server (#264) ----
mssqlDs = envDs("RUSTCFML_TEST_MSSQL_DS");
if ( len(mssqlDs) == 0 ) {
	assertTrue("SQL Server NULL-column check skipped (RUSTCFML_TEST_MSSQL_DS not set)", true);
} else {
	assertNullColEmpty("SQL Server", mssqlDs);
}

// ---- MySQL/MariaDB (regression guard for the original v0.444.0 fix) ----
myDs = envDs("RUSTCFML_TEST_MYSQL_DS");
if ( len(myDs) == 0 ) {
	assertTrue("MySQL NULL-column check skipped (RUSTCFML_TEST_MYSQL_DS not set)", true);
} else {
	assertNullColEmpty("MySQL", myDs);
}

suiteEnd();
</cfscript>
