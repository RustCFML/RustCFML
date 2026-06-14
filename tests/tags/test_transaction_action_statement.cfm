<cfscript>
suiteBegin("Tags: cfscript transaction action statement form");

// ============================================================
// Background  (gap surfaced laddering the Wheels migrator on RustCFML 0.153.0)
// ============================================================
// The cfscript `transaction` statement supports a body-less ATTRIBUTE-STATEMENT
// form that issues an explicit boundary against the current transaction:
//   transaction action="commit";
//   transaction action="rollback";
//   transaction action="begin";
// These are statements (terminated by a semicolon, no `{ ... }` block), and are
// the spelling every Wheels migration template emits inside up()/down():
//   transaction { ... ; transaction action="rollback"; }
//
// RustCFML supports the bare `transaction { ... }` block, and (as of PR #32) the
// attribute form WITH a body `transaction action="begin" { ... }`. But the
// body-less STATEMENT form is parsed as a plain expression — the leading token
// `transaction` is read as a variable reference, so at runtime it throws
// "Variable 'transaction' is undefined" (type=expression). Lucee/ACF/BoxLang
// treat `transaction action="..."` as a transaction-control statement and run
// the block to completion. No datasource is required: with no query inside, the
// commit/rollback/begin are no-ops on both engines, so the surrounding code
// simply runs.
//
// catch-body locals don't persist on every engine (BoxLang discards a nested
// `local` on exit), so each probe records its outcome in a struct FIELD.
// ============================================================

// ---- commit statement inside a transaction block ----
txnState_commit = {flag = false};
try {
	transaction {
		txnState_commit.flag = true;
		transaction action="commit";
	}
} catch (any e) {
	txnState_commit.flag = false;
}
assertTrue("transaction{ ...; transaction action=commit; } runs to completion", txnState_commit.flag);

// ---- rollback statement inside a transaction block ----
txnState_rollback = {flag = false};
try {
	transaction {
		txnState_rollback.flag = true;
		transaction action="rollback";
	}
} catch (any e) {
	txnState_rollback.flag = false;
}
assertTrue("transaction{ ...; transaction action=rollback; } runs to completion", txnState_rollback.flag);

// ---- begin/commit statement pair (no enclosing block) ----
txnState_pair = {flag = false};
try {
	transaction action="begin";
	txnState_pair.flag = true;
	transaction action="commit";
} catch (any e) {
	txnState_pair.flag = false;
}
assertTrue("transaction action=begin; ... transaction action=commit; pair runs to completion", txnState_pair.flag);

// ---- CONTROL: bare transaction{} block (no action statement) ----
// Works on RustCFML and Lucee alike — isolates the gap to the action STATEMENT
// form, not transaction blocks in general.
txnState_bare = {flag = false};
try {
	transaction {
		txnState_bare.flag = true;
	}
} catch (any e) {
	txnState_bare.flag = false;
}
assertTrue("CONTROL: bare transaction{} block runs to completion", txnState_bare.flag);

suiteEnd();
</cfscript>
