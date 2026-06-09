<cfscript>
// ListRest must return the LITERAL substring from element 2 onward —
// preserving interior/trailing empty elements AND the original delimiter
// characters. RustCFML (0.92.0) instead collapses empty elements and
// normalizes the delimiter to a single canonical char.
//
// Verified against Lucee 7 (the compatibility reference):
//   listRest(Reverse("/home/index"),"/") -> "emoh/"   (RustCFML: "emoh")
//   listRest("a/b/","/")                 -> "b/"       (RustCFML: "b")
//   listRest("a/b;c/","/;")              -> "b;c/"     (RustCFML: "b/c")
//   listRest("a,,b,,c")                  -> "b,,c"     (RustCFML: "b,c")
//
// Sibling list fns (ListFirst/ListLast/ListLen) intentionally KEEP the
// empty-collapsing semantics — they are asserted here only as controls
// (same answer on both engines) to guard the test wiring.
suiteBegin("ListRest literal remainder (Lucee parity)");

// --- The Wheels-relevant case: Reverse() + ListRest on a path ---
// A trailing original delimiter must survive in the returned remainder.
assert(
    "listRest preserves trailing delim after reverse",
    listRest(Reverse("/home/index"), "/"),
    "emoh/"
);

// --- Trailing empty element preserved ---
assert("listRest keeps trailing empty element", listRest("a/b/", "/"), "b/");

// --- Original delimiter chars preserved (no normalization to first delim) ---
// Multi-char delimiter set "/;" — Lucee returns the literal remainder with
// the SAME delimiter chars the input used, not all rewritten to "/".
assert("listRest keeps original delimiter chars", listRest("a/b;c/", "/;"), "b;c/");

// --- Interior empty elements preserved ---
assert("listRest keeps interior empty elements", listRest("a,,b,,c"), "b,,c");

// --- Baseline (no empties, single delim): identical on both engines ---
assert("listRest basic remainder", listRest("a,b,c"), "b,c");

// --- Single element -> empty remainder (identical on both engines) ---
assert("listRest single element", listRest("a"), "");

// ============================================================
// CONTROLS — pass on BOTH RustCFML and Lucee (guard the wiring).
// Siblings keep empty-collapsing; do NOT assert divergent behavior here.
// ============================================================
assert("control: listLen collapses empties", listLen("a/b/", "/"), 2);
assert("control: listFirst", listFirst("a/b/", "/"), "a");
assert("control: listLast collapses trailing empty", listLast("a/b/", "/"), "b");

suiteEnd();
</cfscript>
