<cfscript>
suiteBegin("UPDATE affected-rows = rows MATCHED, not rows CHANGED");

// An UPDATE's affected-row count (result.recordCount) must reflect the number of
// rows MATCHED by the WHERE clause, NOT the number whose values actually changed.
// A no-op UPDATE (new values == current values) that matches a row must report 1.
//
// This is the contract Preside's DB session storage relies on: its persist() does
// a blind UPDATE keyed by the session id and, if recordCount == 0, assumes the row
// is gone and mints a brand-new session (INSERT + new cookie). MySQL without the
// CLIENT_FOUND_ROWS capability reported rows CHANGED (0 for a no-op), so two
// requests in the same second with an unchanged session churned out a new session
// every time. Lucee/ACF (MySQL Connector/J default), Postgres, SQL Server and
// SQLite all report rows MATCHED here — this test locks that behaviour in.
dbFile = getTempDirectory() & "/rustcfml_affected_" & createUUID() & ".db";
ds = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbFile };

queryExecute( "create table sess ( id varchar(50), val varchar(50) )", {}, { datasource = ds } );
queryExecute(
      "insert into sess ( id, val ) values ( :id, :val )"
    , { id = "s1", val = "x" }
    , { datasource = ds }
);

// A real change matches + changes the row → 1.
queryExecute(
      "update sess set val = :val where id = :id"
    , { val = "y", id = "s1" }
    , { datasource = ds, result = "changedInfo" }
);
assert( "UPDATE that changes a value reports 1 row affected", changedInfo.recordCount, 1 );

// The decisive case: a no-op UPDATE (val already 'y') that MATCHES the row must
// still report 1 — matched, not changed.
queryExecute(
      "update sess set val = :val where id = :id"
    , { val = "y", id = "s1" }
    , { datasource = ds, result = "noopInfo" }
);
assert( "no-op UPDATE (same values) still reports 1 row MATCHED", noopInfo.recordCount, 1 );

// A WHERE that matches nothing genuinely reports 0.
queryExecute(
      "update sess set val = :val where id = :id"
    , { val = "z", id = "does-not-exist" }
    , { datasource = ds, result = "missInfo" }
);
assert( "UPDATE matching no rows reports 0", missInfo.recordCount, 0 );

if ( fileExists( dbFile ) ) {
	fileDelete( dbFile );
}

suiteEnd();
</cfscript>
