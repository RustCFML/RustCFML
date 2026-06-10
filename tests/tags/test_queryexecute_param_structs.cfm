<cfscript>
suiteBegin("queryExecute: cfqueryparam-style param structs");

// Lucee accepts a cfqueryparam-style struct ({value, cfsqltype, null, ...})
// wherever a queryExecute parameter is expected - positional array entries
// and named-struct entries alike - and binds the struct's VALUE (or NULL
// when null=true). Binding the stringified struct itself is never correct.

tmpfile = getTempDirectory() & "/rustcfml_qe_pstruct_" & createUUID() & ".sqlite";
ds = "sqlite://" & tmpfile;
skip = false;

try {
    queryExecute("CREATE TABLE t (id INTEGER, name TEXT)", [], { datasource: ds });
    queryExecute("INSERT INTO t VALUES (1, 'alpha'), (2, 'beta')", [], { datasource: ds });
} catch (any e) {
    skip = true;
    assertTrue("queryExecute param structs skipped (no sqlite): " & e.message, true);
}
</cfscript>

<cfif NOT skip>

<cfscript>
// --- control: positional param struct already binds the value ---
r = queryExecute("SELECT name FROM t WHERE id = ?",
    [{value: 1, cfsqltype: "cf_sql_integer"}], { datasource: ds });
assert("positional param struct binds value (control)", r.recordCount, 1);
assert("positional param struct row (control)", r.name[1] ?: "", "alpha");

// --- gap: NAMED param struct must bind the value, not the struct ---
r2 = queryExecute("SELECT name FROM t WHERE id = :p",
    {p: {value: 2, cfsqltype: "cf_sql_integer"}}, { datasource: ds });
assert("named param struct binds value", r2.recordCount, 1);
assert("named param struct row", r2.name[1] ?: "", "beta");

// --- control: null=true with a QUOTED key binds SQL NULL today ---
r3 = queryExecute("SELECT (?) IS NULL AS isn FROM t WHERE id = 1",
    [{value: "", cfsqltype: "cf_sql_varchar", "null": true}], { datasource: ds });
assertTrue("quoted null key binds SQL NULL (control)", r3.isn[1] ?: false);

// --- gap: null=true with the BARE key (what cfqueryparam compiles to) ---
// Overlaps the struct-literal null-key test on purpose: the cfquery tag's
// null="true" only works when both the literal key and the binding hold.
r4 = queryExecute("SELECT (?) IS NULL AS isn FROM t WHERE id = 1",
    [{value: "", cfsqltype: "cf_sql_varchar", null: true}], { datasource: ds });
assertTrue("bare null key binds SQL NULL", r4.isn[1] ?: false);
</cfscript>

<!--- gap: cfquery tag null="true" end-to-end (compiles to a param struct) --->
<cfquery name="qn" datasource="#ds#">
    SELECT (<cfqueryparam value="" cfsqltype="cf_sql_varchar" null="true" />) IS NULL AS isn FROM t WHERE id = 1
</cfquery>
<cfscript>
assertTrue("cfquery tag null='true' binds SQL NULL", qn.isn[1] ?: false);

try { fileDelete(tmpfile); } catch (any e) {}
</cfscript>

</cfif>

<cfscript>suiteEnd();</cfscript>
