<cfscript>
// A queryExecute failure must raise a DATABASE-typed exception so
// `catch (database e)` / `<cfcatch type="database">` matches — Lucee/ACF parity.
//
// The SQLite and MySQL adapters already raise CfmlError::database, but the
// PostgreSQL adapter funnelled every query failure through
// PgRunError::from_postgres (crates/cfml-stdlib/src/builtins.rs), which wrapped
// them as CfmlError::runtime — so a database-typed catch never matched and the
// error fell through to `catch (any)`. (pg_sql.rs's two param-rewrite errors
// had the same blind spot.) Fixed in v0.542.0, GitHub #293.
//
// Repro class: framework first-run fallbacks wrap "table may not exist yet"
// probes in <cfcatch type="database">. On RustCFML + Postgres the catch never
// matched, so booting against an empty database threw instead of falling back
// (moopa route_registry_store.cfc / core.cfc had to widen to type="any").
//
// CROSS-ENGINE (GitHub #317): this file could not run on Lucee at all until the
// two structural gates below were added, so from v0.542.0 it was only ever
// verified on RustCFML.
//   1. The SQLite leg is gated on the driver being present. Lucee bundles no
//      SQLite JDBC driver, so `jdbc:sqlite:` raises java.io.IOException
//      ("cannot load class … org.sqlite.JDBC") rather than a database error —
//      which describes the missing driver, not our behaviour.
//   2. The cftransaction legs use `transaction datasource="…"`, a RustCFML
//      extension Lucee rejects at COMPILE time. Inline, that killed the whole
//      template — every assertion in it — uncatchably. They now live in a
//      sibling template reached by include, where the rejection is catchable.
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

// ---- SQLite (control, gated on the driver being available) ----
memDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };
sqliteAvailable = true;
try { queryExecute("SELECT 1", [], {datasource: memDs}); }
catch (any e) { sqliteAvailable = false; }

if ( !sqliteAvailable ) {
	assertTrue("SQLite database-typed catch skipped (no SQLite driver on this engine)", true);
} else {
	assert("SQLite: missing-table error lands in catch (database e)", catchClauseFor(memDs), "database");
}

// ---- PostgreSQL (was the bug: landed in catch (any) as a generic runtime error) ----
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

// ---- cftransaction: see transaction_datasource_ext.cfm for why these are
// isolated. Both legs need the SQLite driver as well as the `datasource`
// attribute, so the whole include is skipped when SQLite is absent. ----
txnLegRan = false;
if ( sqliteAvailable ) {
	try {
		include "transaction_datasource_ext.cfm";
		txnLegRan = true;
	} catch (any e) {
		// Only an engine REJECTING the extension is an acceptable skip. Anything
		// else is a genuine failure and must not be swallowed — rethrow it so the
		// runner reports it rather than silently degrading to a pass.
		if ( !( (e.message ?: "") contains "not allowed" ) ) {
			rethrow;
		}
	}
}
if ( !txnLegRan ) {
	assertTrue("cftransaction datasource= legs skipped (engine lacks SQLite or rejects the attribute)", true);
}

suiteEnd();
</cfscript>
