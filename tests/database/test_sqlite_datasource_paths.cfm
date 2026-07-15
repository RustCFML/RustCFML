<cfscript>
suiteBegin("SQLite datasource paths: inline struct + auto-created parent dir");

// --- Inline STRUCT datasource (ACF/Lucee form) truly means :memory: ---
// Previously the struct was stringified into a filename like
// `{class: org.sqlite.JDBC, connectionString: jdbc:sqlite::memory:}` and SQLite
// created that literal file on disk — losing the :memory: intent. Now the
// struct's connectionString is read, so this is a genuine in-memory DB.
memDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite::memory:" };
q = queryExecute( "select 1 as n", {}, { datasource = memDs } );
assert( "inline struct datasource resolves and runs", q.n, 1 );

// --- Absolute path whose parent directory does NOT yet exist is created ---
// SQLite creates the file but not missing parent dirs; RustCFML now creates the
// directory chain so a configured full path just works (was: "unable to open
// database file").
base = getTempDirectory();
if ( right( base, 1 ) != "/" && right( base, 1 ) != "\" ) { base &= "/"; }
dbDir  = base & "rustcfml_dstest_" & getTickCount();
dbFile = dbDir & "/nested/app.db";
try {
    fileDs = { class: "org.sqlite.JDBC", connectionString: "jdbc:sqlite:" & dbFile };
    queryExecute( "create table t ( id integer )", {}, { datasource = fileDs } );
    queryExecute( "insert into t (id) values (42)", {}, { datasource = fileDs } );
    r = queryExecute( "select id from t", {}, { datasource = fileDs } );
    assert( "query against auto-created-dir sqlite file works", r.id, 42 );
    assertTrue( "the db file was created on disk", fileExists( dbFile ) );
} catch ( any e ) {
    assert( "sqlite path with missing parent dir should not error", "OK", "ERR: " & e.message );
} finally {
    // Best-effort cleanup of the temp tree.
    try { if ( fileExists( dbFile ) ) { fileDelete( dbFile ); } } catch ( any ignore ) {}
    try { directoryDelete( dbDir, true ); } catch ( any ignore ) {}
}

suiteEnd();
</cfscript>
