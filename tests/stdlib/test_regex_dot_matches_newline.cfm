<cfscript>
suiteBegin("Regex: '.' matches a newline in the CFML regex functions");

// ============================================================
// Background
// ============================================================
// CFML's regex functions compile DOTALL: a `.` matches a line terminator.
// Rust's regex crate defaults the other way, and RustCFML inherited that.
//
// The failure mode is silent. `reReplace` returns its SUBJECT UNCHANGED when
// the pattern does not match, so a whole-string pattern against a multi-line
// subject does not error — it hands back the input, and the caller carries on
// with a value that is quietly wrong.
//
// Preside's SqlRunner splits a statement at its `where` and rejoins the halves:
//
//     preClause  = sql.reReplaceNoCase( "^(.*?\swhere)\s.*$", "\1" )
//     postClause = sql.reReplaceNoCase( "^.*?\swhere\s", " " )
//     return preClause & postClause
//
// For a statement whose filter spans several lines, the first pattern never
// matched here, so `preClause` was the WHOLE statement and the rejoin appended
// the tail to it — once per null parameter. MariaDB then rejected the result
// with ERROR 1064, pointing at the second copy of the where clause.
//
// `^` and `$` stay SINGLE-line (not multiline) — Lucee agrees, so they are
// asserted here too, to pin the half of the behaviour that must NOT change.
// ============================================================

nl = chr( 10 );
twoLines = "a" & nl & "b";

// --- the gap: '.' crosses a line terminator ---
assert( "reReplace: '.' matches a newline", reReplace( twoLines, "a.b", "X" ), "X" );
assert( "reReplaceNoCase: '.' matches a newline", reReplaceNoCase( twoLines, "A.B", "X" ), "X" );
assertTrue( "reFind: '.' matches a newline", reFind( "a.b", twoLines ) == 1 );
assertTrue( "reFindNoCase: '.' matches a newline", reFindNoCase( "A.B", twoLines ) == 1 );
assert( "reMatch: '.' matches a newline", arrayLen( reMatch( "a.b", twoLines ) ), 1 );

// A whole-string anchored pattern reaches the end of a multi-line subject.
sql = "select a from t where (x = 1" & nl & "   OR y = 2" & nl & "   OR z = 3) limit 5";
assert( "an anchored pattern spans every line",
    reReplaceNoCase( sql, "^(.*?\swhere)\s.*$", "\1" ), "select a from t where" );

// Splitting a multi-line statement at its `where` and rejoining must be a
// round trip — this is the exact Preside shape that produced doubled SQL.
preClause  = reReplaceNoCase( sql, "^(.*?\swhere)\s.*$", "\1" );
postClause = reReplaceNoCase( sql, "^.*?\swhere\s", " " );
assert( "split-and-rejoin round-trips a multi-line statement", preClause & postClause, sql );

// --- and the half that must NOT change: ^ and $ stay single-line ---
assert( "'$' does not match before an inner newline", reFind( "a$", twoLines ), 0 );
assert( "'^' does not match after an inner newline", reFind( "^b", twoLines ), 0 );

// An explicit `(?-s)` still turns dot-matches-newline back off.
assert( "(?-s) restores the non-DOTALL meaning", reReplace( twoLines, "(?-s)a.b", "X" ), twoLines );

suiteEnd();
</cfscript>
