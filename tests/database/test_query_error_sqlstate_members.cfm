<cfscript>
// GitHub #295 — a database exception must carry the structured detail Lucee/ACF
// attach, not just a message. Follow-up to #293 (which typed the exception) and
// to test_query_error_catch_type_database.cfm (which asserts the typing).
//
// Reference behaviour, verified against Lucee 7.0.4 driving pgjdbc, MariaDB
// Connector/J and mssql-jdbc — `SELECT * FROM <missing>` yields:
//
//   member           PostgreSQL   MySQL/MariaDB   SQL Server
//   type             database     database        database
//   SQLState         42P01        42S02           S0002   (jdbc-synthesised)
//   NativeErrorCode  0            1146            208
//   ErrorCode        0            0               0
//   Detail           ""           ""              ""      (empty on ALL drivers)
//   where            ""           ""              ""
//   Sql / queryError <the statement>
//   DataSource       <ds name; "__temp__" for an inline struct datasource>
//
// Note ErrorCode is NOT the vendor code on Lucee — it is a literal 0, and the
// vendor number lives in NativeErrorCode. The two are asserted separately below
// so a future change can't quietly collapse them.
//
// SQLite has no SQLSTATE (and Lucee ships no SQLite driver, so there is no
// reference); it reports the extended result code in NativeErrorCode and leaves
// SQLState empty. MSSQL's SQLSTATE is an mssql-jdbc invention that tiberius does
// not expose, so it is left empty too. Both are recorded in docs/known-issues.md.
//
// PostgreSQL/MySQL legs are live-gated (same convention as
// test_query_error_catch_type_database.cfm) and skip when the env var is unset:
//   docker run -d -p 55432:5432 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=testdb postgres:16
//   RUSTCFML_TEST_PG_DS=postgres://postgres:test@127.0.0.1:55432/testdb \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("database exceptions expose sqlState and friends (##295)");

function envDs(required string varName) {
	try { return getEnvironmentVariable(arguments.varName, ""); }
	catch (any e) { return ""; }
}

// Run `sql` against `ds` and return the caught exception as a plain struct of
// the members under test (empty string for anything absent), so an assertion
// failure names the member rather than blowing up with "variable undefined".
function memberSnapshot(required any ds, required string sql) {
	try {
		queryExecute(arguments.sql, [], {datasource: arguments.ds});
		return {caught: false};
	} catch (any e) {
		return {
			  caught          : true
			, type            : lCase(e.type ?: "")
			, sqlState        : e.sqlState ?: ""
			, nativeErrorCode : e.nativeErrorCode ?: ""
			, errorCode       : e.errorCode ?: ""
			, detail          : e.detail ?: ""
			, where           : e.where ?: ""
			, sql             : e.sql ?: ""
			, queryError      : e.queryError ?: ""
			, datasource      : e.datasource ?: ""
		};
	}
}

MISSING_TABLE_SQL = "SELECT 1 FROM rcfml_missing_table_gh295";

// ---- SQLite (no-network control) -----------------------------------------
// Gated on the driver actually being present. RustCFML has SQLite compiled in,
// so this leg always runs here; Lucee bundles no SQLite JDBC driver, so the
// probe below fails to load org.sqlite.JDBC and the leg skips rather than
// spraying reds that describe the missing driver, not our behaviour.
memDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };
sqliteAvailable = true;
try { queryExecute("SELECT 1", [], {datasource: memDs}); }
catch (any e) { sqliteAvailable = false; }

if ( !sqliteAvailable ) {
	assertTrue("SQLite sqlState members skipped (no SQLite driver on this engine)", true);
} else {

s = memberSnapshot(memDs, MISSING_TABLE_SQL);

assertTrue("SQLite: an error was raised", s.caught);
assert("SQLite: type is database", s.type, "database");
// Every member must EXIST (that is the whole point of #295) even where the
// driver has nothing to put in it — a handler reading e.sqlState unguarded must
// not get a secondary "Variable SQLSTATE is undefined" (cf. GitHub #250).
assertTrue("SQLite: sqlState member exists", structKeyExists(s, "sqlState"));
assert("SQLite: sqlState is empty (SQLite has no SQLSTATE)", s.sqlState, "");
// SQLITE_ERROR = 1; a missing table is a plain parse/prepare failure.
assert("SQLite: nativeErrorCode is the extended result code", s.nativeErrorCode, "1");
assert("SQLite: errorCode is 0 (Lucee parity — vendor code lives in nativeErrorCode)", s.errorCode, "0");
assert("SQLite: detail is empty (Lucee parity)", s.detail, "");
assert("SQLite: where is empty", s.where, "");
assert("SQLite: sql is the failing statement", s.sql, MISSING_TABLE_SQL);
assert("SQLite: queryError mirrors sql", s.queryError, MISSING_TABLE_SQL);
assert("SQLite: datasource is the inline struct sentinel", s.datasource, "__temp__");

// A named datasource reports its NAME, not its connection string.
assertTrue(
	"SQLite: a raw-URL datasource never reports credentials",
	!find("@", memberSnapshot("jdbc:sqlite::memory:", MISSING_TABLE_SQL).datasource)
);

// Constraint violations are the reason sqlState/nativeErrorCode matter: the
// caller wants to distinguish "already exists" from "table missing" WITHOUT
// substring-matching a driver message.
// DROP first: in serve mode the `:memory:` datasource is pooled and outlives the
// request, so a bare CREATE fails on the suite's second run.
queryExecute("DROP TABLE IF EXISTS gh295_uniq", [], {datasource: memDs});
queryExecute("CREATE TABLE gh295_uniq (id integer primary key)", [], {datasource: memDs});
queryExecute("INSERT INTO gh295_uniq (id) VALUES (1)", [], {datasource: memDs});
dupe = memberSnapshot(memDs, "INSERT INTO gh295_uniq (id) VALUES (1)");
assertTrue("SQLite: duplicate-key insert raised", dupe.caught);
// SQLITE_CONSTRAINT_PRIMARYKEY = 1555 — distinct from the 1 above, which is
// exactly the discrimination the issue asks for.
assert("SQLite: constraint violation has its own nativeErrorCode", dupe.nativeErrorCode, "1555");
assertTrue(
	"SQLite: the two failures are distinguishable without reading the message",
	dupe.nativeErrorCode != s.nativeErrorCode
);

} // end SQLite leg

// ---- PostgreSQL (live-gated) ---------------------------------------------
pgDs = envDs("RUSTCFML_TEST_PG_DS");
if ( len(pgDs) == 0 ) {
	assertTrue("PostgreSQL sqlState members skipped (RUSTCFML_TEST_PG_DS not set)", true);
} else {
	pg = memberSnapshot(pgDs, MISSING_TABLE_SQL);
	assertTrue("PostgreSQL: an error was raised", pg.caught);
	assert("PostgreSQL: type is database", pg.type, "database");
	// The headline assertion of #295: the portable SQLSTATE for undefined_table.
	assert("PostgreSQL: sqlState is 42P01 (undefined_table)", pg.sqlState, "42P01");
	assert("PostgreSQL: errorCode is 0 (Lucee parity)", pg.errorCode, "0");
	assert("PostgreSQL: nativeErrorCode is 0 (PG has no vendor code)", pg.nativeErrorCode, "0");
	assert("PostgreSQL: detail is empty (Lucee parity)", pg.detail, "");
	assert("PostgreSQL: sql is the failing statement", pg.sql, MISSING_TABLE_SQL);
	assertTrue("PostgreSQL: datasource never carries credentials", !find("@", pg.datasource));

	// Syntax error and unique violation carry DIFFERENT states, so a caller can
	// branch on them — the real-world driver in the issue.
	syntaxErr = memberSnapshot(pgDs, "SELEKT 1");
	assert("PostgreSQL: sqlState is 42601 for a syntax error", syntaxErr.sqlState, "42601");

	try { queryExecute("DROP TABLE IF EXISTS gh295_pg_uniq", [], {datasource: pgDs}); } catch (any e) {}
	queryExecute("CREATE TABLE gh295_pg_uniq (id int primary key)", [], {datasource: pgDs});
	queryExecute("INSERT INTO gh295_pg_uniq (id) VALUES (1)", [], {datasource: pgDs});
	pgDupe = memberSnapshot(pgDs, "INSERT INTO gh295_pg_uniq (id) VALUES (1)");
	assert("PostgreSQL: sqlState is 23505 for a unique violation", pgDupe.sqlState, "23505");
	queryExecute("DROP TABLE gh295_pg_uniq", [], {datasource: pgDs});
}

// ---- MySQL / MariaDB (live-gated) ----------------------------------------
myDs = envDs("RUSTCFML_TEST_MYSQL_DS");
if ( len(myDs) == 0 ) {
	assertTrue("MySQL sqlState members skipped (RUSTCFML_TEST_MYSQL_DS not set)", true);
} else {
	my = memberSnapshot(myDs, MISSING_TABLE_SQL);
	assertTrue("MySQL: an error was raised", my.caught);
	assert("MySQL: type is database", my.type, "database");
	assert("MySQL: sqlState is 42S02 (base table not found)", my.sqlState, "42S02");
	// Unlike PG, MySQL DOES have a vendor number, and Lucee reports it here.
	assert("MySQL: nativeErrorCode is 1146", my.nativeErrorCode, "1146");
	assert("MySQL: errorCode is 0 (Lucee parity)", my.errorCode, "0");
	assert("MySQL: sql is the failing statement", my.sql, MISSING_TABLE_SQL);
	assertTrue("MySQL: datasource never carries credentials", !find("@", my.datasource));

	syntaxErr = memberSnapshot(myDs, "SELEKT 1");
	assert("MySQL: sqlState is 42000 for a syntax error", syntaxErr.sqlState, "42000");
	assert("MySQL: nativeErrorCode is 1064 for a syntax error", syntaxErr.nativeErrorCode, "1064");
}

// ---- SQL Server (live-gated) ---------------------------------------------
// Lucee reports SQLState "S0002" here, but that is a legacy ODBC state
// mssql-jdbc synthesises — SQL Server's wire protocol carries no SQLSTATE, and
// tiberius exposes none. We report the vendor number and leave SQLState empty
// rather than invent a fourth convention (docs/known-issues.md).
msDs = envDs("RUSTCFML_TEST_MSSQL_DS");
if ( len(msDs) == 0 ) {
	assertTrue("MSSQL sqlState members skipped (RUSTCFML_TEST_MSSQL_DS not set)", true);
} else {
	ms = memberSnapshot(msDs, MISSING_TABLE_SQL);
	assertTrue("MSSQL: an error was raised", ms.caught);
	assert("MSSQL: type is database", ms.type, "database");
	assert("MSSQL: nativeErrorCode is 208 (invalid object name)", ms.nativeErrorCode, "208");
	assert("MSSQL: errorCode is 0 (Lucee parity)", ms.errorCode, "0");
	assert("MSSQL: sqlState is empty (tiberius exposes no SQLSTATE)", ms.sqlState, "");
	assert("MSSQL: sql is the failing statement", ms.sql, MISSING_TABLE_SQL);
	assertTrue("MSSQL: datasource never carries credentials", !find("@", ms.datasource));
}

suiteEnd();
</cfscript>
