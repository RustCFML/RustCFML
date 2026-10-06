<cfscript>
suiteBegin("docs/markdown.md worked examples");

// The two worked examples in docs/markdown.md, run as written. Their expected
// output lives in fixtures/markdown/ and is the same text the doc shows, so a
// change to the engine that would make the doc wrong fails here.
fixtures = getDirectoryFromPath( getCurrentTemplatePath() ) & "fixtures/markdown/";

// ---- a report from a query -------------------------------------------------
orders = queryNew(
    "ref,customer,placed,status,total,notes",
    "varchar,varchar,date,varchar,double,varchar",
    [
        [ "A-1001", "Acme_Corp", createDate( 2026, 9, 2 ),  "paid",    1240.5, "" ],
        [ "A-1002", "C* Labs",   createDate( 2026, 9, 14 ), "overdue", 980,    "Chased **twice**, see [ticket 412](https://help.example/412)" ],
        [ "A-1003", "Globex",    createDate( 2026, 9, 27 ), "paid",    15200,  "" ]
    ]
);

report = MarkdownDocument()
    .heading( "September orders" )
    .paragraph( [
        "Prepared for ",
        { text = "Finance", bold = true },
        " on ",
        { text = dateFormat( createDate( 2026, 10, 1 ), "d mmmm yyyy" ) },
        "."
    ] )
    .paragraph( "Totals include VAT. Questions go to [the help desk](https://help.example)." )
    .heading( "All orders", 2 )
    .table(
        data          = orders,
        columnList    = "ref,customer,placed,total,notes",
        headers       = "Ref,Customer,Placed,**Total**,Notes",
        columnFormats = {
            placed = { dateFormat = "d mmm" },
            total  = { numberFormat = "9,999.00", align = "right" },
            notes  = { format = "markdown" }
        }
    )
    .heading( "Next steps", 2 )
    .markdown( "
        - Chase overdue orders by **Friday**
        - Close the month

        > Owner: Finance operations
    " );

overdue = queryExecute( "select ref from orders where status = 'overdue'", {}, { dbtype = "query" } );

report
    .appendToSection( "All orders", "#overdue.recordCount# order is overdue: **#markdownEscape( overdue.ref )#**." )
    .moveSectionBefore( "Next steps", "All orders" );

assert( "the report's markdown is what the doc shows", report.toMarkdown(), fileRead( fixtures & "report_expected.md" ) );
outline = report.outline();
assert( "outline: three headings", arrayLen( outline ), 3 );
assert( "Next steps moved before All orders", outline[ 2 ].text & "|" & outline[ 3 ].text, "Next steps|All orders" );
assert( "at the positions the doc shows", outline[ 2 ].position & "," & outline[ 3 ].position, "4,7" );
html = report.toHtml();
assertTrue( "literal data renders as typed", findNoCase( "<td>Acme_Corp</td>", html ) > 0 && findNoCase( "<td>C* Labs</td>", html ) > 0 );
assertTrue( "the markdown column renders", findNoCase( "Chased <strong>twice</strong>", html ) > 0 );

// ---- editing an existing file ------------------------------------------------
path = getTempDirectory() & "rustcfml_md_example_" & createUUID() & ".md";
fileWrite( path, fileRead( fixtures & "post_before.md" ) );

post = MarkdownDocument( fileRead( path ) );
fixes = post.section( "v2.1" ).ids[ 2 ];
post
    .setFrontMatter( { title = "Release notes", draft = false } )
    .insertBefore( "v2.1", MarkdownDocument().heading( "v2.2", 2 ).list( [ "Markdown export" ] ) )
    .append( fixes, "Fixed: the date filter" );
fileWrite( path, post.toMarkdown() );

assert( "the edited file is what the doc shows", fileRead( path ), fileRead( fixtures & "post_expected.md" ) );
fileDelete( path );

suiteEnd();
</cfscript>
