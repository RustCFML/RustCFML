<cfscript>
// SQL functions provided by an extension, inside Query-of-Queries.
posts = queryNew(
      "title,views"
    , "varchar,integer"
    , [ [ "Hello, World!",        10 ]
      , [ "Rust & CFML: a pair",  90 ]
      , [ "  Trailing  spaces  ", 50 ]
      , [ "Fourth Post",          30 ] ]
);

writeOutput( "— a SCALAR, called once per row —" & chr(10) );
slugs = queryExecute(
      "SELECT title, SLUGIFY( title ) AS slug FROM posts ORDER BY views DESC"
    , {}
    , { dbtype = "query" }
);
for ( row in slugs ) {
    writeOutput( "  " & row.slug & chr(10) );
}

writeOutput( chr(10) & "— an AGGREGATE, called once per partition —" & chr(10) );
stats = queryExecute(
      "SELECT MEDIAN( views ) AS mid, COUNT( * ) AS n FROM posts"
    , {}
    , { dbtype = "query" }
);
writeOutput( "  median views across " & stats.n & " rows = " & stats.mid & chr(10) );

writeOutput( chr(10) & "— and the same functions are ordinary BIFs —" & chr(10) );
writeOutput( "  slugify( 'Hello, World!' ) = " & slugify( "Hello, World!" ) & chr(10) );
</cfscript>
