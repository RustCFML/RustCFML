<cfscript>
// The PostgreSQL placeholder rewriters treat an apostrophe inside a SQL
// comment as a string OPENER. rewrite_positional / rewrite_named
// (crates/cfml-stdlib/src/pg_sql.rs) skip quoted strings and dollar-quoted
// bodies but NOT `--` / `/* */` comments — while split_sql_statements in the
// SAME file skips all three. So a contraction in a comment ("it's", "don't")
// opens a phantom string literal and everything up to the next apostrophe or
// end of statement — including `?` placeholders — is copied through as string
// body, and the statement under-counts its parameters:
//
//   queryExecute: 2 positional parameter(s) supplied but the SQL only consumes 1
//
// The named rewriter has a second failure mode needing no apostrophe at all:
// a bare `:word` inside a comment is rewritten to `$n` and consumes a
// parameter slot, so binding fails (or every later bind shifts by one).
//
// Repro class: English contractions in SQL comments are everywhere. A titan
// dashboard query with six `?` binds died on a `-- ... project's margin ...`
// annotation — the error points at the parameter count, nowhere near the
// comment, so it presents as a baffling caller-side bug. Lucee/ACF hand the
// SQL to a driver that parses comments correctly, so this only bites here.
//
// Live test, gated on RUSTCFML_TEST_PG_DS (same convention as
// test_query_error_catch_type_database.cfm; skipped when unset). To run:
//   docker run -d -p 55432:5432 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=testdb postgres:16
//   RUSTCFML_TEST_PG_DS=postgres://postgres:test@127.0.0.1:55432/testdb \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("PG placeholder scan: comments are not string regions (live; skipped without DS env var)");

function envDs(required string varName) {
	try { return getEnvironmentVariable(arguments.varName, ""); }
	catch (any e) { return ""; }
}

// Run a two-column SELECT and normalize the outcome: either the row values or
// the error. queryExecute on the broken engine throws (param under-count /
// unknown named param), so every leg must be catchable or the first failure
// would abort the whole template.
function selectAB(required string sql, required any params, required string ds) {
	try {
		var q = queryExecute(arguments.sql, arguments.params, {datasource: arguments.ds});
		return "a=" & q.a[1] & ";b=" & q.b[1];
	} catch (any e) {
		return "ERROR: " & (e.message ?: "");
	}
}

pgDs = envDs("RUSTCFML_TEST_PG_DS");
if ( len(pgDs) == 0 ) {
	assertTrue("PG comment/placeholder scan skipped (RUSTCFML_TEST_PG_DS not set)", true);
} else {
	nl = chr(10);

	// Control: a comment WITHOUT an apostrophe or :word never confused the
	// scanner — this leg passing while the others fail localizes the bug to
	// comment-body content, not comment handling per se.
	assert(
		"control: positional binds with a plain line comment",
		selectAB("SELECT ?::int AS a -- plain comment" & nl & ", ?::int AS b", [1, 2], pgDs),
		"a=1;b=2"
	);

	// An apostrophe in a `--` comment must not swallow the `?` after it.
	assert(
		"positional `?` after an apostrophe in a line comment",
		selectAB("SELECT ?::int AS a -- it's a comment" & nl & ", ?::int AS b", [1, 2], pgDs),
		"a=1;b=2"
	);

	// Same, block comment: /* don't */ sits between the two binds.
	assert(
		"positional `?` after an apostrophe in a block comment",
		selectAB("SELECT ?::int AS a, /* don't panic */ ?::int AS b", [1, 2], pgDs),
		"a=1;b=2"
	);

	// Named binds after an apostrophe in a line comment.
	assert(
		"named `:b` after an apostrophe in a line comment",
		selectAB("SELECT CAST(:a AS int) AS a -- it's a comment" & nl & ", CAST(:b AS int) AS b", {a: 1, b: 2}, pgDs),
		"a=1;b=2"
	);

	// No apostrophe needed for the named rewriter: a bare `:word` inside a
	// comment must not be taken as a placeholder (here it would demand a third
	// parameter that the caller — correctly — never supplied).
	assert(
		"`:word` inside a line comment is not a placeholder",
		selectAB("SELECT CAST(:a AS int) AS a, CAST(:b AS int) AS b -- see :note for details", {a: 1, b: 2}, pgDs),
		"a=1;b=2"
	);

	assert(
		"`:word` inside a block comment is not a placeholder",
		selectAB("SELECT CAST(:a AS int) AS a, /* cf. :note */ CAST(:b AS int) AS b", {a: 1, b: 2}, pgDs),
		"a=1;b=2"
	);
}

suiteEnd();
</cfscript>
