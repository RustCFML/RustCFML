<cfscript>
suiteBegin("Tags: nested cftransaction blocks (savepoint semantics)");

// ============================================================
// Background  (gap surfaced laddering the Wheels framework test suite on RustCFML 0.220.0–0.228.0)
// ============================================================
// A `transaction { ... }` block may be NESTED inside another `transaction { ... }`
// block. Lucee/ACF/BoxLang implement the inner block as a SAVEPOINT on the
// already-open outer transaction, so the code runs to completion. RustCFML
// instead throws at the inner block:
//   "cftransaction: nested transactions are not supported"
//
// Wheels relies on this heavily: model save()/create() and the migrator wrap
// work in a transaction, and application code frequently opens its own
// transaction around several saves — producing a transaction inside a
// transaction. The Wheels core test suite hits this in 84 specs; the suite is
// green on Lucee. On RustCFML every model create() performed inside an outer
// transaction aborts here.
//
// No datasource is required: with no query inside, the outer begin and the
// inner savepoint are no-ops on Lucee/ACF/BoxLang, so the surrounding code
// simply runs (same rationale as test_transaction_action_statement.cfm).
//
// catch-body locals don't persist on every engine (BoxLang discards a nested
// `local` on exit), so each probe records its outcome in a struct FIELD.
// ============================================================

// ---- a transaction block nested inside another transaction block ----
nestedState = {inner = false, outer = false, err = ""};
try {
	transaction {
		transaction {
			nestedState.inner = true;
		}
		nestedState.outer = true;
	}
} catch (any e) {
	nestedState.err = e.message;
}
assertTrue("inner nested transaction{} block runs to completion", nestedState.inner);
assertTrue("outer transaction{} block continues after the nested block", nestedState.outer);

// ---- CONTROL: a single (non-nested) transaction block still works ----
bareState = {flag = false};
try {
	transaction {
		bareState.flag = true;
	}
} catch (any e) {
	bareState.flag = false;
}
assertTrue("CONTROL: single transaction{} block runs to completion", bareState.flag);

suiteEnd();
</cfscript>
