<cfscript>
// A queryExecute failure must raise a DATABASE-typed exception so
// `catch (database e)` / `<cfcatch type="database">` matches — Lucee/ACF parity.
//
// The SQLite and MySQL adapters already raise CfmlError::database, but the
// PostgreSQL adapter funnels every query failure through
// PgRunError::from_postgres (crates/cfml-stdlib/src/builtins.rs), which wraps
// them as CfmlError::runtime — so a database-typed catch never matches and the
// error falls through to `catch (any)`. (pg_sql.rs's two param-rewrite errors
// have the same blind spot.)
//
// Repro class: framework first-run fallbacks wrap "table may not exist yet"
// probes in <cfcatch type="database">. On RustCFML + Postgres the catch never
// matches, so booting against an empty database throws instead of falling back
// (moopa route_registry_store.cfc / core.cfc had to widen to type="any").
//
// The PostgreSQL leg is live-gated (same convention as
// test_db_null_column_empty_string.cfm) and skips when the env var is unset:
//   docker run -d -p 55432:5432 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=testdb postgres:16
//   RUSTCFML_TEST_PG_DS=postgres://postgres:test@127.0.0.1:55432/testdb \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("queryExecute failures are catchable as type=database");

function envDs(required string varName) {
	try { return getEnvironmentVariable(arguments.varName, ""); }
	catch (any e) { return ""; }
}

// Which catch clause does a missing-table query error land in?
function catchClauseFor(required any ds) {
	try {
		queryExecute("SELECT 1 FROM rcfml_missing_table_f362", [], {datasource: arguments.ds});
		return "no-error-raised";
	} catch (database e) {
		return "database";
	} catch (any e) {
		return "any (cfcatch.type=" & (e.type ?: "") & ")";
	}
}

// ---- SQLite (control: already lands in the database-typed catch) ----
memDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };
assert("SQLite: missing-table error lands in catch (database e)", catchClauseFor(memDs), "database");

// ---- PostgreSQL (currently lands in catch (any) as a generic runtime error) ----
pgDs = envDs("RUSTCFML_TEST_PG_DS");
if ( len(pgDs) == 0 ) {
	assertTrue("PostgreSQL database-typed catch skipped (RUSTCFML_TEST_PG_DS not set)", true);
} else {
	assert("PostgreSQL: missing-table error lands in catch (database e)", catchClauseFor(pgDs), "database");

	// Lucee parity detail: the caught exception reports its type as "database".
	caughtType = "";
	try {
		queryExecute("SELECT 1 FROM rcfml_missing_table_f362", [], {datasource: pgDs});
	} catch (any e) {
		caughtType = lCase(e.type ?: "");
	}
	assert("PostgreSQL: cfcatch.type is database", caughtType, "database");
}

suiteEnd();
</cfscript>
