<cfscript>
// cfqueryparam list="true" with an ARRAY value: Lucee coerces the array and
// expands one bind per element — `value="#someArray#"` inside `IN (...)` is
// a standard Lucee idiom. RustCFML instead serializes the whole array as ONE
// parameter, and the failure mode differs by driver:
//
//   QoQ:        SILENTLY WRONG — `a IN (?)` with [2,4] matches 0 rows,
//               no error raised.
//   PostgreSQL: throws — cannot bind "[<uuid>]" as PostgreSQL uuid:
//               invalid character: found `[` at 0.
//
// The silent QoQ leg is the dangerous one: a search that returns nothing
// looks like "no results", not like a bug.
//
// Repro class: titan (Moopa) binds pg_trgm search results with exactly this
// shape at 21 call sites — `id IN (<cfqueryparam cfsqltype="other"
// list="true" value="#idsInSearchTerm(...)#">)` where idsInSearchTerm
// returns an ARRAY of uuids. Every search screen died (or silently emptied)
// on this engine until each site was wrapped in arrayToList().
//
// The PostgreSQL leg is live-gated on RUSTCFML_TEST_PG_DS (same convention
// as test_query_error_catch_type_database.cfm):
//   docker run -d -p 55432:5432 -e POSTGRES_PASSWORD=test -e POSTGRES_DB=testdb postgres:16
//   RUSTCFML_TEST_PG_DS=postgres://postgres:test@127.0.0.1:55432/testdb \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("cfqueryparam list=""true"": an array value expands one bind per element");

function envDs(required string varName) {
	try { return getEnvironmentVariable(arguments.varName, ""); }
	catch (any e) { return ""; }
}

src = queryNew("a", "integer", [ [1], [2], [3], [4] ]);
idsArr = [ 2, 4 ];
oneId = [ 3 ];
</cfscript>

<!--- Control: a string list expands on both engines. --->
<cftry>
    <cfquery name="qStr" dbtype="query">SELECT a FROM src WHERE a IN (<cfqueryparam cfsqltype="integer" list="true" value="#arrayToList(idsArr)#" />)</cfquery>
    <cfset strResult = qStr.recordcount />
    <cfcatch type="any"><cfset strResult = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<!--- The gap: the same values as an ARRAY. --->
<cftry>
    <cfquery name="qArr" dbtype="query">SELECT a FROM src WHERE a IN (<cfqueryparam cfsqltype="integer" list="true" value="#idsArr#" />)</cfquery>
    <cfset arrResult = qArr.recordcount />
    <cfcatch type="any"><cfset arrResult = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<!--- Single-element array: the shape a no-match sentinel produces. --->
<cftry>
    <cfquery name="qOne" dbtype="query">SELECT a FROM src WHERE a IN (<cfqueryparam cfsqltype="integer" list="true" value="#oneId#" />)</cfquery>
    <cfset oneResult = qOne.recordcount />
    <cfcatch type="any"><cfset oneResult = "THREW: " & cfcatch.message /></cfcatch>
</cftry>

<cfscript>
assert( "control: string list matches both rows", strResult, 2 );
assert( "array value matches the same two rows (saw: " & arrResult & ")", arrResult, 2 );
assert( "single-element array matches its row", oneResult, 1 );

// ── PostgreSQL leg: the uuid shape that throws rather than silently missing ──
pgDs = envDs("RUSTCFML_TEST_PG_DS");
</cfscript>

<cfif len(pgDs) EQ 0>
    <cfscript>assertTrue( "PostgreSQL array-list leg skipped (RUSTCFML_TEST_PG_DS not set)", true );</cfscript>
<cfelse>
    <cfset uuidArr = [ "607ceee8-2cc0-4f9a-bed8-9f2f3affc575", "9fd8e0be-57ce-4aa5-aa2f-492d4808094c" ] />
    <cftry>
        <cfquery name="qPg" datasource="#pgDs#">
            SELECT 1 AS hit
            WHERE '607ceee8-2cc0-4f9a-bed8-9f2f3affc575'::uuid IN (<cfqueryparam cfsqltype="other" list="true" value="#uuidArr#" />)
        </cfquery>
        <cfset pgResult = qPg.recordcount />
        <cfcatch type="any"><cfset pgResult = "THREW: " & cfcatch.message /></cfcatch>
    </cftry>
    <cfscript>assert( "PostgreSQL: uuid array expands into the IN list (saw: " & pgResult & ")", pgResult, 1 );</cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
