<cfscript>
// <cfprocparam type="out|inout" variable="x"> assigns the procedure's output to
// x. The type and variable attributes used to be dropped, so an OUT parameter
// was bound as an ordinary input and its variable never set.
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS (see test_cfstoredproc_result_sets.cfm).
// SQL Server takes a different route to the same values (one batch with
// DECLARE … OUTPUT); see run_proc_with_outputs.

suiteBegin("cfprocparam OUT/INOUT (skipped without RUSTCFML_TEST_MYSQL_DS)");

poDsn = "";
try {
    poDsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    poDsn = "";
}

if ( !len( poDsn ) ) {
    assertTrue("skipped: RUSTCFML_TEST_MYSQL_DS not set", true);
} else {
    poProc = "zz_rcfml_out_" & lCase( left( replace( createUUID(), "-", "", "all" ), 8 ) );
    queryExecute( "CREATE PROCEDURE #poProc#(IN a INT, OUT doubled INT, INOUT counter INT, OUT label VARCHAR(20), OUT nothing INT) BEGIN SELECT a AS a_in; SET doubled = a * 2; SET counter = counter + 1; SET label = 'hi'; SET nothing = NULL; END", [], { datasource: poDsn } );

    function poLocalOut( required string proc, required string dsn ) {
        var localDoubled = 0;
        cfstoredproc( procedure=arguments.proc, datasource=arguments.dsn ) {
            cfprocparam( cfsqltype="cf_sql_integer", value=21 );
            cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="localDoubled" );
            cfprocparam( type="inout", cfsqltype="cf_sql_integer", value=0, variable="localCounter" );
            cfprocparam( type="out", cfsqltype="cf_sql_varchar", variable="localLabel" );
            cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="localNothing" );
        }
        return localDoubled;
    }

    try {
        include "_test_cfprocparam_out_tags1.cfm";
        assert("tag: OUT value", tagDoubled, 10);
        assert("tag: INOUT value goes in and comes back", tagCounter, 42);
        assert("tag: OUT string", tagLabel, "hi");
        assert("tag: NULL OUT reads as empty string", tagNothing, "");
        assert("tag: the procedure's result set still comes back", tagRows.a_in, 5);
        assert("runtime body: OUT value", runtimeDoubled, 14);
        assert("runtime body: INOUT value", runtimeCounter, 2);

        cfstoredproc( procedure=poProc, datasource=poDsn ) {
            cfprocparam( cfsqltype="cf_sql_integer", value=3 );
            cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="scriptDoubled" );
            cfprocparam( type="inout", cfsqltype="cf_sql_integer", value=9, variable="scriptCounter" );
            cfprocparam( type="out", cfsqltype="cf_sql_varchar", variable="scriptLabel" );
            cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="scriptNothing" );
            cfprocresult( name="scriptRows" );
        }
        assert("script: OUT value", scriptDoubled, 6);
        assert("script: INOUT value", scriptCounter, 10);
        assert("script: result set", scriptRows.a_in, 3);

        transaction {
            cfstoredproc( procedure=poProc, datasource=poDsn ) {
                cfprocparam( cfsqltype="cf_sql_integer", value=50 );
                cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="txnDoubled" );
                cfprocparam( type="inout", cfsqltype="cf_sql_integer", value=0, variable="txnCounter" );
                cfprocparam( type="out", cfsqltype="cf_sql_varchar", variable="txnLabel" );
                cfprocparam( type="out", cfsqltype="cf_sql_integer", variable="txnNothing" );
            }
        }
        assert("in a transaction: OUT value", txnDoubled, 100);

        assert("an OUT variable declared with var stays local", poLocalOut( poProc, poDsn ), 42);
        assert("so it does not leak into variables", structKeyExists( variables, "localDoubled" ), false);
        assert("the call temporaries do not leak", structKeyExists( variables, "__cfproc_r" ), false);
    } finally {
        queryExecute( "DROP PROCEDURE IF EXISTS #poProc#", [], { datasource: poDsn } );
    }
}

suiteEnd();
</cfscript>
