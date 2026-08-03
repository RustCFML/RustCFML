<cfscript>
suiteBegin("Tags: cfscript transaction with attributes + body");

// ============================================================
// Background  (parse gap surfaced in PR #32 by bpamiri)
// ============================================================
// The cfscript `transaction` statement accepts space-separated tag attributes
// before its body, mirroring the angle-bracket <cftransaction action="..."> tag:
//   transaction action="begin" { ... }
// RustCFML supported the bare `transaction { ... }` form but rejected the
// attribute form ("Expected RBrace, found Semicolon"). Lucee/Adobe CF/BoxLang
// accept both. A transaction with no query inside is a no-op on both engines,
// so the body simply runs.
// ============================================================

function runAttrTxn() {
	var marker = "";
	transaction action="begin" {
		marker = "ran";
	}
	return marker;
}
assert("transaction action=begin executes its body", runAttrTxn(), "ran");

function runBareTxn() {
	var n = 0;
	transaction {
		n = 42;
	}
	return n;
}
assert("bare transaction executes its body (regression guard)", runBareTxn(), 42);

// ============================================================
// isolation= must not be mistaken for the datasource
// ============================================================
// `__cftransaction_start` takes (action, isolation, datasource) positionally,
// but the lowerings used to emit only the attributes actually present. With
// `isolation` alone the value landed in the DATASOURCE slot, so the block tried
// to open a connection to a datasource literally named "serializable" and threw
// instead of running the body. All three forms are covered — the angle-bracket
// tag, the script block, and the script statement. (`isolation` itself is still
// not applied to the connection — docs/known-issues.md §7.)
function runIsolationBlockTxn() {
	var marker = "";
	transaction isolation="serializable" {
		marker = "ran";
	}
	return marker;
}
assert("transaction isolation= runs its body (block form)", runIsolationBlockTxn(), "ran");

function runIsolationStatementTxn() {
	transaction action="begin" isolation="read_committed";
	var marker = "ran";
	transaction action="commit";
	return marker;
}
assert("transaction isolation= runs (statement form)", runIsolationStatementTxn(), "ran");
</cfscript>

<cftry>
	<cftransaction isolation="serializable">
		<cfset tagIsolationMarker = "ran">
	</cftransaction>
	<cfcatch>
		<cfscript> tagIsolationMarker = "THREW: " & cfcatch.message; </cfscript>
	</cfcatch>
</cftry>

<cfscript>
assert("cftransaction isolation= runs its body (tag form)", tagIsolationMarker, "ran");

suiteEnd();
</cfscript>
