<cfscript>
// <cfprocparam null="true"> binds NULL. The attribute used to be dropped by the
// tag lowering, so the value ("" when absent) reached the database instead:
// SQL Server stored 0 in an int column, and a NULL guard in the procedure
// never fired.
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS (see test_cfstoredproc_result_sets.cfm).

suiteBegin("cfprocparam null (skipped without RUSTCFML_TEST_MYSQL_DS)");

pnDsn = "";
try {
    pnDsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    pnDsn = "";
}

if ( !len( pnDsn ) ) {
    assertTrue("skipped: RUSTCFML_TEST_MYSQL_DS not set", true);
} else {
    pnProc = "zz_rcfml_isnull_" & lCase( left( replace( createUUID(), "-", "", "all" ), 8 ) );
    queryExecute( "CREATE PROCEDURE #pnProc#(IN a INT, IN b VARCHAR(20)) BEGIN SELECT a IS NULL AS a_null, b IS NULL AS b_null, a AS a_value; END", [], { datasource: pnDsn } );
    try {
        pnEmpty = "";
</cfscript>
<cfstoredproc procedure="#pnProc#" datasource="#pnDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="#pnEmpty#" null="#( NOT len( pnEmpty ) )#">
    <cfprocparam cfsqltype="cf_sql_varchar" null="true">
    <cfprocresult name="tagNull">
</cfstoredproc>
<cfstoredproc procedure="#pnProc#" datasource="#pnDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="5" null="false">
    <cfprocparam cfsqltype="cf_sql_varchar" value="x" null="no">
    <cfprocresult name="tagValue">
</cfstoredproc>
<cfstoredproc procedure="#pnProc#" datasource="#pnDsn#">
    <cfif true><cfprocparam cfsqltype="cf_sql_integer" value="" null="yes"></cfif>
    <cfprocparam cfsqltype="cf_sql_varchar" null="1">
    <cfprocresult name="runtimeNull">
</cfstoredproc>
<cfscript>
        assert("tag: null=expression true binds NULL", tagNull.a_null, 1);
        assert("tag: null=true with no value binds NULL", tagNull.b_null, 1);
        assert("tag: null=false binds the value", tagValue.a_value, 5);
        assert("tag: null=no binds the value", tagValue.b_null, 0);
        assert("runtime body: null=yes binds NULL", runtimeNull.a_null, 1);
        assert("runtime body: null=1 binds NULL", runtimeNull.b_null, 1);

        cfstoredproc( procedure=pnProc, datasource=pnDsn ) {
            cfprocparam( cfsqltype="cf_sql_integer", null=true );
            cfprocparam( cfsqltype="cf_sql_varchar", value="y", null=false );
            cfprocresult( name="scriptResult" );
        }
        assert("script: null=true with no value binds NULL", scriptResult.a_null, 1);
        assert("script: null=false binds the value", scriptResult.b_null, 0);
    } finally {
        queryExecute( "DROP PROCEDURE IF EXISTS #pnProc#", [], { datasource: pnDsn } );
    }
}

suiteEnd();
</cfscript>
