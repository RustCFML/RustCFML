<cfscript>
// Included by test_query_error_catch_type_database.cfm — NOT a standalone test
// (no suiteBegin/suiteEnd, and it must not be listed in tests/runner.cfm).
//
// These assertions live in their own template because they use
// `transaction datasource="…"`, which is a RustCFML extension: Lucee 7.0.4
// rejects `datasource` on cftransaction in BOTH the script and tag forms
// ("Valid Attribute names are [action, isolation, savepoint]"). Forwarding it
// through all three transaction lowerings was deliberate — docs/known-issues.md
// §27b — so the engine is not what diverges here; the syntax simply has no
// Lucee equivalent.
//
// The isolation matters because Lucee's rejection happens at COMPILE time, in
// the transformer. Inline in the parent file it took out the whole template —
// every assertion in it, not just these two — and no try/cfcatch could trap it,
// which is how the file stayed un-run on Lucee from v0.542.0 (GitHub #317).
// Reached via `include`, the same rejection arrives as a catchable error at
// include time, so the caller can skip this leg and keep the rest of the suite.
//
// `memDs` is inherited from the including template's variables scope.

// Pool checkout / BEGIN / COMMIT / ROLLBACK / savepoint failures are database
// failures too, so they must arrive database-typed.
function txnCatchClauseFor( required any ds, required string sql ) {
	try {
		transaction datasource="#arguments.ds#" {
			queryExecute( arguments.sql, [], {datasource: arguments.ds} );
		}
		return "no-error-raised";
	} catch (database e) {
		return "database";
	} catch (any e) {
		return "any (cfcatch.type=" & (e.type ?: "") & ")";
	}
}

assert(
	"cftransaction: a failing statement is database-typed",
	txnCatchClauseFor(memDs, "SELECT 1 FROM rcfml_missing_table_f362"),
	"database"
);
assert(
	"cftransaction: an unopenable datasource is database-typed",
	txnCatchClauseFor(
		{ class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:/rcfml-no-such-dir-f362/t.db" },
		"SELECT 1"
	),
	"database"
);
</cfscript>
