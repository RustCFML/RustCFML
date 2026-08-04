<cfscript>
suiteBegin("A non-SELECT statement returns an empty query (§37)");

// INSERT / UPDATE / DELETE must RETURN an empty query, not the mutation-metadata
// struct. Verified against Lucee 7.0.4 over MySQL: each hands back
// QUERY(recordCount=0), and the affected-row count / generated key are exposed
// ONLY through the `result=` struct.
//
// RustCFML returned that metadata struct directly, so a `query`-declared function
// wrapping a DELETE failed §29 type enforcement:
//
//   private query function _deleteSessionRecord( required string sessionId ) {
//       return sqlRunner.runSql( sql = "delete from psys_session_storage where id = :id", ... );
//   }
//
// -> "The function [_deleteSessionRecord] has an invalid return value ,
//     [Cannot cast Object type [Struct] to a value of type [query]]"
//
// That is Preside's SessionStorage, and it broke the admin route.
//
// Driven on SQLite so no server is needed. Lucee ships no SQLite JDBC driver, so
// the whole suite is skipped there with one informational pass rather than
// spraying false reds — the cross-engine evidence for this behaviour was taken on
// MySQL, where both engines agree exactly.
dbFile = getTempDirectory() & "/rustcfml_dml_" & createUUID() & ".db";
ds = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbFile };

driverAvailable = true;
try {
    queryExecute( "create table dml ( id int primary key, v varchar(20) )", {}, { datasource = ds } );
} catch ( any e ) {
    driverAvailable = false;
}

if ( !driverAvailable ) {
    assertTrue( "skipped — no SQLite JDBC driver on this engine", true );
} else {
    // --- the return value of each statement kind ---------------------------
    ins = queryExecute( "insert into dml ( id, v ) values ( 1, 'a' )", {}, { datasource = ds } );
    assertTrue( "INSERT returns a query",  isQuery( ins ) );
    assert(     "INSERT query is empty",   ins.recordCount, 0 );

    upd = queryExecute( "update dml set v = 'b' where id = 1", {}, { datasource = ds } );
    assertTrue( "UPDATE returns a query",  isQuery( upd ) );
    assert(     "UPDATE query is empty",   upd.recordCount, 0 );

    del = queryExecute( "delete from dml where id = 1", {}, { datasource = ds } );
    assertTrue( "DELETE returns a query",  isQuery( del ) );
    assert(     "DELETE query is empty",   del.recordCount, 0 );

    // Explicitly NOT a struct — that is the whole point.
    assertFalse( "DELETE does not return a struct", isStruct( del ) );

    // --- a query-declared function may wrap a mutation ---------------------
    // The Preside shape. Before the fix this threw.
    queryExecute( "insert into dml ( id, v ) values ( 2, 'c' )", {}, { datasource = ds } );
    query function deleteTyped() {
        return queryExecute( "delete from dml where id = 2", {}, { datasource = ds } );
    }
    assertTrue( "a query-declared function may return a DELETE", isQuery( deleteTyped() ) );

    // --- result= must STILL carry the mutation metadata -------------------
    // The metadata struct is the internal carrier for this; converting the
    // RETURN value must not have cost us the `result=` contract.
    queryExecute( "insert into dml ( id, v ) values ( 3, 'd' )", {}, { datasource = ds, result = "insRes" } );
    assert( "result= reports rows affected by an INSERT", insRes.recordCount, 1 );

    queryExecute( "update dml set v = 'e' where id = 3", {}, { datasource = ds, result = "updRes" } );
    assert( "result= reports rows affected by an UPDATE", updRes.recordCount, 1 );

    queryExecute( "delete from dml where id = 3", {}, { datasource = ds, result = "delRes" } );
    assert( "result= reports rows affected by a DELETE", delRes.recordCount, 1 );

    // --- a SELECT is untouched -------------------------------------------
    queryExecute( "insert into dml ( id, v ) values ( 4, 'f' )", {}, { datasource = ds } );
    sel = queryExecute( "select id, v from dml", {}, { datasource = ds } );
    assertTrue( "SELECT still returns a query", isQuery( sel ) );
    assert(     "SELECT still returns its rows", sel.recordCount, 1 );
    assert(     "SELECT rows are readable",      sel.v, "f" );

    // A returntype="struct" SELECT must not be mistaken for mutation metadata
    // (it is a struct too — the discriminator is the executionTime/cached pair).
    selStruct = queryExecute(
          "select id, v from dml"
        , {}
        , { datasource = ds, returntype = "struct", columnkey = "id" }
    );
    assertTrue( "returntype=struct SELECT still returns a struct", isStruct( selStruct ) );
    assertTrue( "returntype=struct SELECT is keyed by its columnkey", structKeyExists( selStruct, "4" ) );

    try { fileDelete( dbFile ); } catch ( any e ) {}
}

suiteEnd();
</cfscript>
