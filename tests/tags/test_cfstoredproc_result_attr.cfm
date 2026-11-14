<cfscript>
// cfstoredproc result="x" sets { executionTime, statusCode }; with
// returnCode="true", statusCode is the procedure's return value. Both
// attributes used to be ignored, so reading sproc.statusCode threw. Lucee 7.1
// sets no `cfstoredproc` variable, so neither does this.
//
// MySQL procedures have no return value, so statusCode is 0 here; SQL Server
// reads it with EXEC @rc = proc (see run_proc_with_outputs).
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS (see test_cfstoredproc_result_sets.cfm).

suiteBegin("cfstoredproc result and returnCode (skipped without RUSTCFML_TEST_MYSQL_DS)");

raDsn = "";
try {
    raDsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    raDsn = "";
}

if ( !len( raDsn ) ) {
    assertTrue("skipped: RUSTCFML_TEST_MYSQL_DS not set", true);
} else {
    raProc = "zz_rcfml_rc_" & lCase( left( replace( createUUID(), "-", "", "all" ), 8 ) );
    queryExecute( "CREATE PROCEDURE #raProc#(IN a INT, OUT b INT) BEGIN SET b = a + 1; SELECT a AS a_in; END", [], { datasource: raDsn } );
    try {
        raHolder = {};
</cfscript>
<cfstoredproc procedure="#raProc#" datasource="#raDsn#" result="tagResult" returnCode="true">
    <cfprocparam cfsqltype="cf_sql_integer" value="1">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="tagB">
    <cfprocresult name="tagRows">
</cfstoredproc>
<cfstoredproc procedure="#raProc#" datasource="#raDsn#" result="raHolder.sproc">
    <cfprocparam cfsqltype="cf_sql_integer" value="2">
    <cfprocparam type="out" cfsqltype="cf_sql_integer" variable="plainB">
</cfstoredproc>
<cfscript>
        assert("tag: result.statusCode", tagResult.statusCode, 0);
        assertTrue("tag: result.executionTime is a number", isNumeric( tagResult.executionTime ));
        assert("no cfstoredproc variable, as on Lucee", isNull( variables.cfstoredproc ), true);
        assert("tag: OUT value alongside", tagB, 2);
        assert("tag: result set alongside", tagRows.a_in, 1);
        assert("tag: a dotted result target", raHolder.sproc.statusCode, 0);
        assert("tag: OUT value without returnCode", plainB, 3);

        cfstoredproc( procedure=raProc, datasource=raDsn, result="scriptResult", returnCode=true ) {
            cfprocparam( cfsqltype="cf_sql_integer", value=5 );
            cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="scriptB" );
        }
        assert("script: result.statusCode", scriptResult.statusCode, 0);
        assertTrue("script: result.executionTime is a number", isNumeric( scriptResult.executionTime ));
        assert("script: OUT value", scriptB, 6);
        assert("the timer temporary does not leak", structKeyExists( variables, "__cfproc_t" ), false);
    } finally {
        queryExecute( "DROP PROCEDURE IF EXISTS #raProc#", [], { datasource: raDsn } );
    }
}

suiteEnd();
</cfscript>
