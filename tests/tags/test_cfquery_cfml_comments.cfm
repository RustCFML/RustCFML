<cfscript>
// CFML comments inside a cfquery BODY never reach the database: Lucee strips
// <!--- ---> lexically at compile time, wherever it appears — annotating SQL
// with CFML comments is idiomatic and safe there. RustCFML passes them
// through to the driver:
//
//   PostgreSQL:  prepare error — ERROR: operator does not exist: <! numeric
//                (or: syntax error at or near "<!")
//   QoQ:         Query of Queries syntax error: unexpected token in
//                expression: Error("unexpected '!'")  [inline]
//                / unexpected token in expression: Lt  [multi-line]
//
// Repro class: titan (Moopa) annotates pricing SQL with CFML comments —
// 88 comments across 13 route files, all fine on Lucee for years. On
// RustCFML the sale-edit lines query died the first time it ran, and every
// one of the 88 had to be stripped app-side.
//
// Note the contrast pinned by the controls: SQL `--` comments are DATA and
// must reach the engine/driver (both engines agree), and a CFML comment in a
// queryExecute STRING is string content, not markup — only the tag BODY is
// CFML-lexed. The PostgreSQL leg is live-gated on RUSTCFML_TEST_PG_DS (same
// convention as test_query_error_catch_type_database.cfm):
//   docker run -d -p 55432:5432 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=testdb postgres:16
//   RUSTCFML_TEST_PG_DS=postgres://postgres:test@127.0.0.1:55432/testdb \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("cfquery body: CFML comments are stripped before the SQL is executed");

function envDs(required string varName) {
	try { return getEnvironmentVariable(arguments.varName, ""); }
	catch (any e) { return ""; }
}

src = queryNew("a,b", "integer,integer", [ [1, 2] ]);
</cfscript>

<!--- ── QoQ legs: engine-parsed, cross-engine, no external DB ── --->
<cftry>
    <cfquery name="qInline" dbtype="query">SELECT a <!--- pricing note: markup from base cost ---> , b FROM src</cfquery>
    <cfset inlineResult = "#qInline.a#/#qInline.b#" />
    <cfcatch type="any"><cfset inlineResult = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cftry>
    <cfquery name="qMulti" dbtype="query">
        SELECT a,
        <!--- a multi-line comment,
              the way real annotations wrap --->
        b FROM src
    </cfquery>
    <cfset multiResult = "#qMulti.a#/#qMulti.b#" />
    <cfcatch type="any"><cfset multiResult = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<!--- Control: a SQL -- comment is data and must SURVIVE (both engines agree).
      Newline-terminated: Lucee's QoQ parser errors on a line comment that
      ends at EOF, which is a different quirk than the one under test. --->
<cftry>
    <cfquery name="qSqlCmt" dbtype="query">
        SELECT a, b -- sql comment, kept
        FROM src
    </cfquery>
    <cfset sqlCmtResult = "#qSqlCmt.a#/#qSqlCmt.b#" />
    <cfcatch type="any"><cfset sqlCmtResult = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "QoQ: inline CFML comment in the body is stripped", inlineResult, "1/2" );
assert( "QoQ: multi-line CFML comment in the body is stripped", multiResult, "1/2" );
assert( "control: a SQL -- comment in the body still executes", sqlCmtResult, "1/2" );

// ── PostgreSQL leg: the real-world shape (prepare on a live server) ──
pgDs = envDs("RUSTCFML_TEST_PG_DS");
</cfscript>

<cfif len(pgDs) EQ 0>
    <cfscript>assertTrue( "PostgreSQL CFML-comment leg skipped (RUSTCFML_TEST_PG_DS not set)", true );</cfscript>
<cfelse>
    <cftry>
        <cfquery name="qPg" datasource="#pgDs#">SELECT 1 AS a <!--- cfml annotation ---> , 2 AS b</cfquery>
        <cfset pgResult = "#qPg.a#/#qPg.b#" />
        <cfcatch type="any"><cfset pgResult = "THREW: " & cfcatch.message /></cfcatch>
    </cftry>
    <cfscript>assert( "PostgreSQL: CFML comment in a tag body never reaches the server", pgResult, "1/2" );</cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
