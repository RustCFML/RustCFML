<cfscript>
suiteBegin("Types: bare query-column access returns a native-typed scalar (numeric preserved)");

// ============================================================
// Background
// ============================================================
// Accessing a query column WITHOUT a row index (q.col / q["col"]) returns the
// value at the CURRENT row (row 1 outside a cfloop) as a native simple value
// carrying the column's type. On Lucee/Adobe CF/BoxLang a numeric column read
// this way IsNumeric() and participates in arithmetic. The row-INDEXED form
// (q.col[1]) is equivalent.
//
// RustCFML 0.153.0 returns the right string for bare access (q.n stringifies
// to "5") but it is NOT numeric-typed: IsNumeric(q.n) is false and q.n + 1
// concatenates ("51") instead of adding (6). The row-indexed form q.n[1] IS
// numeric on RustCFML, so the gap is specifically the BARE (current-row
// scalar) access losing the native numeric type. Holds for queryNew, QoQ
// (queryExecute and cfquery), and datasource queries alike.
//
// Why it matters for Wheels: model.count() / sum() / average() / minimum() /
// maximum() all end in vendor/wheels/model/calculations.cfc::$calculate with
//   local.rv = local.rv[local.alias];   // BARE scalar read of the aggregate
// and count() then guards `if (!IsNumeric(local.rv)) local.rv = 0;`. On
// RustCFML the bare aggregate is non-numeric, so the guard forces every count
// to 0 — which makes findAll(page=,perPage=) compute zero total pages and
// return NO rows, breaking every paginated index. Surfaced laddering pagination.
// ============================================================

qbcQ = queryNew("n,label", "integer,varchar", [{n: 5, label: "x"}, {n: 9, label: "y"}]);

// --- the gap: bare column access is numeric-typed ---
assertTrue("bare q.col on an integer column IsNumeric", isNumeric(qbcQ.n));
assert("bare q.col participates in arithmetic (not string concat)", qbcQ.n + 1, 6);
assertTrue("bare q['col'] on an integer column IsNumeric", isNumeric(qbcQ["n"]));

// --- a QoQ COUNT aggregate read bare (the exact Wheels $calculate shape) ---
qbcC = queryExecute("SELECT COUNT(*) AS cnt FROM qbcQ", {}, {dbtype: "query"});
assertTrue("bare aggregate (COUNT) read IsNumeric", isNumeric(qbcC.cnt));
assert("bare COUNT participates in arithmetic", qbcC.cnt + 0, 2);

// --- CONTROL (green on both engines): the row-indexed form is numeric ---
assertTrue("CONTROL: row-indexed q.col[1] IsNumeric", isNumeric(qbcQ.n[1]));
assert("CONTROL: row-indexed arithmetic", qbcQ.n[1] + 1, 6);

suiteEnd();
</cfscript>
