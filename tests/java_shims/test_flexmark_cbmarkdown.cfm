<cfscript>
suiteBegin("java shim: flexmark (cbmarkdown) over the built-in markdown engine");

// ColdBox's cbmarkdown module — vendored into Preside extensions, so it turns
// up in most Preside apps — builds a flexmark pipeline through cbjavaloader at
// module load. With no JVM every one of those calls used to raise the
// deferred-Java error, and since it happens in a module's init() it took the
// whole application's boot with it. The classes are shimmed onto RustCFML's own
// markdown engine instead, so cbmarkdown needs no source change.
//
// This mirrors cbmarkdown's Processor.cfc init()/createOptions() call for call.

h = chr( 35 ); // a literal hash, which CFML strings would otherwise eat

loader = createObject( "java", "java.net.URLClassLoader" );

StaticParser         = loader.loadClass( "com.vladsch.flexmark.parser.Parser" );
HtmlRenderer         = loader.loadClass( "com.vladsch.flexmark.html.HtmlRenderer" );
staticTableExtension = loader.loadClass( "com.vladsch.flexmark.ext.tables.TablesExtension" );
anchorLinkExtension  = loader.loadClass( "com.vladsch.flexmark.ext.anchorlink.AnchorLinkExtension" );

// --- the static option fields read as plain members ---
assert( "a table option key reads as a token", staticTableExtension.CLASS_NAME, "tables.CLASS_NAME" );
assert( "a parser option key reads as a token", StaticParser.EXTENSIONS, "parser.EXTENSIONS" );
assertTrue( "an anchor-link option key reads as a token",
    len( anchorLinkExtension.ANCHORLINKS_SET_ID ) > 0 );

// --- X.create() builds the extension markers ---
extensionsToLoad = [
      staticTableExtension.create()
    , loader.loadClass( "com.vladsch.flexmark.ext.gfm.strikethrough.StrikethroughSubscriptExtension" ).create()
    , loader.loadClass( "com.vladsch.flexmark.ext.gfm.tasklist.TaskListExtension" ).create()
    , loader.loadClass( "com.vladsch.flexmark.ext.toc.TocExtension" ).create()
];
assert( "every extension was created", arrayLen( extensionsToLoad ), 4 );

// --- MutableDataSet.set() is chainable, as flexmark's is ---
parserOptions = loader.loadClass( "com.vladsch.flexmark.util.data.MutableDataSet" ).init()
    .set( StaticParser.WWW_AUTO_LINK_ELEMENT, javacast( "boolean", true ) )
    .set( anchorLinkExtension.ANCHORLINKS_SET_ID, javacast( "boolean", true ) )
    .set( staticTableExtension.CLASS_NAME, "table table-striped" )
    .set( StaticParser.EXTENSIONS, extensionsToLoad );
assertFalse( "the option chain returns the data set", isNull( parserOptions ) );

parser   = StaticParser.builder( parserOptions ).build();
renderer = HtmlRenderer.builder( parserOptions ).build();

// --- markdown in, HTML out ---
md = h & h & " Hello" & chr( 10 ) & chr( 10 )
   & "Some **bold** and ~~struck~~ text." & chr( 10 ) & chr( 10 )
   & "| a | b |" & chr( 10 ) & "|---|---|" & chr( 10 ) & "| 1 | 2 |";

html = renderer.render( parser.parse( md ) );

assertTrue( "the heading renders at its own level", findNoCase( "<h2", html ) > 0 );
assertTrue( "bold renders", findNoCase( "<strong>bold</strong>", html ) > 0 );
assertTrue( "the strikethrough extension is on", findNoCase( "<del>struck</del>", html ) > 0 );
assertTrue( "the tables extension is on", findNoCase( "<table", html ) > 0 );
// TablesExtension.CLASS_NAME reaches the rendered table.
assertTrue( "the table carries the configured class",
    findNoCase( 'class="table table-striped"', html ) > 0 );
// ANCHORLINKS_SET_ID reaches the headings.
assertTrue( "headings get ids when anchor links are enabled", findNoCase( 'id="hello"', html ) > 0 );

// flexmark passes raw HTML through, so the shim does too — an app that wired
// up its own markdown pipeline is trusted input.
rawHtml = renderer.render( parser.parse( "<div class=""raw"">kept</div>" ) );
assertTrue( "raw HTML is passed through", findNoCase( "<div class=""raw"">kept</div>", rawHtml ) > 0 );

// --- HTML in, markdown out (FlexmarkHtmlConverter) ---
htmlToMarkdown = loader.loadClass( "com.vladsch.flexmark.html2md.converter.FlexmarkHtmlConverter" )
    .builder( parserOptions )
    .build();
backToMd = htmlToMarkdown.convert( "<h2>Hi</h2><p>Some <b>bold</b></p>" );
assertTrue( "the heading round-trips", findNoCase( h & h & " Hi", backToMd ) > 0 );
assertTrue( "the bold round-trips", findNoCase( "**bold**", backToMd ) > 0 );

// A renderer handed a plain string (rather than a parsed document) still works.
assertTrue( "the renderer accepts a bare string",
    findNoCase( "<em>x</em>", renderer.render( "*x*" ) ) > 0 );

suiteEnd();
</cfscript>
