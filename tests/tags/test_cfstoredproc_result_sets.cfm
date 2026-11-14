<cfscript>
// <cfprocresult resultSet="n"> takes the procedure's nth result set. Every
// cfprocresult used to receive result set 1, so a second one held the first
// set's columns. A set the procedure doesn't return leaves its variable unset.
//
// Live test, gated on RUSTCFML_TEST_MYSQL_DS (a MySQL/MariaDB procedure returns
// one result set per SELECT). SQL Server takes the same path; see
// test_mssql_dml_output.cfm for running a SQL Server container. To run locally:
//   docker run --rm -d -p 3306:3306 -e MYSQL_ROOT_PASSWORD=root -e MYSQL_DATABASE=test mysql:8.4
//   RUSTCFML_TEST_MYSQL_DS=mysql://root:root@127.0.0.1:3306/test \
//     cargo run --features all-databases -- tests/runner.cfm

suiteBegin("cfprocresult resultSet (skipped without RUSTCFML_TEST_MYSQL_DS)");

rsDsn = "";
try {
    rsDsn = server.system.environment.RUSTCFML_TEST_MYSQL_DS ?: "";
} catch (any e) {
    rsDsn = "";
}

if ( !len( rsDsn ) ) {
    assertTrue("skipped: RUSTCFML_TEST_MYSQL_DS not set", true);
} else {
    rsSuffix = lCase( left( replace( createUUID(), "-", "", "all" ), 8 ) );
    rsSets = "zz_rcfml_sets_" & rsSuffix;
    rsNone = "zz_rcfml_nosets_" & rsSuffix;
    queryExecute( "CREATE PROCEDURE #rsSets#(IN n INT) BEGIN SELECT n AS a, 'x' AS b; SELECT 1 AS c UNION ALL SELECT 2; SELECT 'none' AS e FROM DUAL WHERE 1 = 0; END", [], { datasource: rsDsn } );
    queryExecute( "CREATE PROCEDURE #rsNone#() BEGIN DO 1; END", [], { datasource: rsDsn } );
    try {
</cfscript>
<cfstoredproc procedure="#rsSets#" datasource="#rsDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="7">
    <cfprocresult name="tagSecond" resultSet="2">
    <cfprocresult name="tagFirst">
    <cfprocresult name="tagThird" resultSet="3">
    <cfprocresult name="tagMissing" resultSet="4">
</cfstoredproc>
<cfscript>
        assert("tag: resultSet 1 by default", tagFirst.columnList, "A,B");
        assert("tag: resultSet 1 value", tagFirst.a, 7);
        assert("tag: resultSet 2 is the second SELECT", tagSecond.columnList, "C");
        assert("tag: resultSet 2 rows", tagSecond.recordCount, 2);
        assert("tag: an empty result set keeps its columns", tagThird.columnList, "E");
        assert("tag: an empty result set has no rows", tagThird.recordCount, 0);
        assert("tag: a set the procedure didn't return is unset", isNull( tagMissing ), true);

        cfstoredproc( procedure=rsSets, datasource=rsDsn ) {
            cfprocparam( cfsqltype="cf_sql_integer", value=8 );
            cfprocresult( name="scriptSecond", resultSet=2 );
            cfprocresult( name="scriptFirst", resultSet=1 );
        }
        assert("script: resultSet 1", scriptFirst.a, 8);
        assert("script: resultSet 2", scriptSecond.recordCount, 2);

        // maxRows caps the rows kept; -1 is no limit (Lucee).
        rsOne = 1;
</cfscript>
<cfstoredproc procedure="#rsSets#" datasource="#rsDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="7">
    <cfprocresult name="tagCapped" resultSet="2" maxRows="#rsOne#">
    <cfprocresult name="tagUncapped" resultSet="2" maxRows="-1">
</cfstoredproc>
<cfscript>
        assert("tag: maxRows caps a result set", tagCapped.recordCount, 1);
        assert("tag: maxRows keeps the first rows", tagCapped.c, 1);
        assert("tag: maxRows=-1 is no limit", tagUncapped.recordCount, 2);
        cfstoredproc( procedure=rsSets, datasource=rsDsn ) {
            cfprocparam( cfsqltype="cf_sql_integer", value=8 );
            cfprocresult( name="scriptCapped", resultSet=2, maxRows=rsOne );
        }
        assert("script: maxRows caps a result set", scriptCapped.recordCount, 1);
        // maxRows=0 keeps no rows, but the columns.
        cfstoredproc( procedure=rsSets, datasource=rsDsn ) {
            cfprocparam( cfsqltype="cf_sql_integer", value=8 );
            cfprocresult( name="scriptNone", resultSet=2, maxRows=0 );
        }
        assert("script: maxRows=0 keeps no rows", scriptNone.recordCount, 0);
        assert("script: maxRows=0 keeps the columns", scriptNone.columnList, "C");
</cfscript>
<cfstoredproc procedure="#rsSets#" datasource="#rsDsn#">
    <cfprocparam cfsqltype="cf_sql_integer" value="7">
    <cfprocresult name="tagNone" resultSet="2" maxRows="0">
</cfstoredproc>
<cfscript>
        assert("tag: maxRows=0 keeps no rows", tagNone.recordCount, 0);
        assert("tag: maxRows=0 keeps the columns", tagNone.columnList, "C");

        transaction {
            cfstoredproc( procedure=rsSets, datasource=rsDsn ) {
                cfprocparam( cfsqltype="cf_sql_integer", value=9 );
                cfprocresult( name="txnSecond", resultSet=2 );
            }
        }
        assert("in a transaction: resultSet 2", txnSecond.columnList, "C");

        cfstoredproc( procedure=rsNone, datasource=rsDsn ) {
            cfprocresult( name="noneFirst" );
        }
        assert("a procedure with no result set leaves the variable unset", isNull( noneFirst ), true);

        assert("the result-set temporary does not leak into variables",
            structKeyExists( variables, "__cfproc_rs" ), false);
    } finally {
        queryExecute( "DROP PROCEDURE IF EXISTS #rsSets#", [], { datasource: rsDsn } );
        queryExecute( "DROP PROCEDURE IF EXISTS #rsNone#", [], { datasource: rsDsn } );
    }
}

suiteEnd();
</cfscript>
