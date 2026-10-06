<cfscript>
suiteBegin("markdown(), htmlToMarkdown(), markdownEscape(), <cfmarkdown>");

// A `#` in a CFML string literal is doubled, so "## Title" is the markdown
// "# Title". chr(10) is a newline.
nl = chr(10);

// ---- markdown(): BoxLang's bx-markdown shape -------------------------------
assert( "a heading", trim( markdown( "## Title" ) ), "<h1>Title</h1>" );
assert( "inline markup", trim( markdown( "Pay **now**, _please_ and see `x`." ) ),
	"<p>Pay <strong>now</strong>, <em>please</em> and see <code>x</code>.</p>" );
assert( "named argument is BoxLang's name", trim( markdown( markdown = "*hi*" ) ), "<p><em>hi</em></p>" );
assert( "named arguments in any order",
	trim( markdown( options = { hardBreaks = true }, markdown = "a" & nl & "b" ) ), "<p>a<br />" & nl & "b</p>" );
assert( "an empty string is empty", markdown( "" ), "" );

// GFM is on by default: tables, strikethrough, task lists, autolinks.
html = markdown( "| a | b |" & nl & "|---|--:|" & nl & "| 1 | 2 |" );
assertTrue( "a table", findNoCase( "<table>", html ) > 0 );
assertTrue( "with column alignment", findNoCase( '<td align="right">2</td>', html ) > 0 );
assertTrue( "strikethrough", findNoCase( "<del>gone</del>", markdown( "~~gone~~" ) ) > 0 );
assertTrue( "task list", findNoCase( 'type="checkbox"', markdown( "- [x] done" ) ) > 0 );
assertTrue( "autolink", findNoCase( '<a href="https://example.com">', markdown( "see https://example.com" ) ) > 0 );
assertFalse( "gfm can be turned off", findNoCase( "<del>", markdown( "~~gone~~", { gfm = false } ) ) > 0 );

// ---- safe by default --------------------------------------------------------
// Raw HTML is dropped and dangerous URL schemes are blanked. BoxLang passes raw
// HTML through; { unsafe = true } (or markdown.unsafe in .cfconfig.json) matches it.
safe = markdown( "<b>raw</b> [x](javascript:alert(1)) <script>alert(1)</script>" );
assertFalse( "raw HTML is dropped", findNoCase( "<b>", safe ) > 0 );
assertFalse( "script is dropped", findNoCase( "<script", safe ) > 0 );
assertTrue( "a javascript: link is blanked", findNoCase( '<a href="">x</a>', safe ) > 0 );
assertTrue( "unsafe passes raw HTML", findNoCase( "<b>raw</b>", markdown( "<b>raw</b>", { unsafe = true } ) ) > 0 );
assertTrue( "unsafe keeps the url", findNoCase( "javascript:", markdown( "[x](javascript:void(0))", { unsafe = true } ) ) > 0 );
assertTrue( "escapeHtml shows it as text", findNoCase( "&lt;b&gt;raw&lt;/b&gt;", markdown( "<b>raw</b>", { escapeHtml = true } ) ) > 0 );

// ---- options ----------------------------------------------------------------
assertTrue( "anchors give headings ids", findNoCase( 'id="hello-world"', markdown( "## Hello world", { anchors = true } ) ) > 0 );
assertTrue( "tableClass", findNoCase( '<table class="table">', markdown( "| a |" & nl & "|---|" & nl & "| 1 |", { tableClass = "table" } ) ) > 0 );
assertTrue( "footnotes", findNoCase( "footnote", markdown( "a[^1]" & nl & nl & "[^1]: note", { footnotes = true } ) ) > 0 );
// Markdown indented to match the code around it is not a code block when dedented.
indented = "    ## Title" & nl & "    Body";
assertTrue( "indented markdown is a code block by default", findNoCase( "<pre>", markdown( indented ) ) > 0 );
assert( "dedent fixes that", trim( markdown( indented, { dedent = true } ) ), "<h1>Title</h1>" & nl & "<p>Body</p>" );
assertTrue( "front matter is kept out of the body", findNoCase( "title:", markdown( "---" & nl & "title: x" & nl & "---" & nl & "Body" ) ) == 0 );
assertThrows( "an unknown option is an error", function(){ markdown( "x", { bogus = true } ); } );
assertThrows( "options must be a struct", function(){ markdown( "x", "nope" ); } );

// ---- markdownEscape(): data into a markdown string ----------------------------
name = "C* and __init__ <b>";
assert( "escaped data renders literally", trim( markdown( "Customer: " & markdownEscape( name ) ) ),
	"<p>Customer: C* and __init__ &lt;b&gt;</p>" );
assert( "a list marker stays text", trim( markdown( markdownEscape( "1. not a list" ) ) ), "<p>1. not a list</p>" );
assert( "a heading marker stays text", trim( markdown( markdownEscape( "## not a heading" ) ) ), "<p>## not a heading</p>" );
assertFalse( "an autolink stays text", findNoCase( "<a ", markdown( markdownEscape( "www.example.com" ) ) ) > 0 );

// ---- htmlToMarkdown(): BoxLang's shape -----------------------------------------
md = htmlToMarkdown( "<h2>Title</h2><p>Some <strong>bold</strong> and <a href='https://x.test'>a link</a>.</p><ul><li>one</li><li>two</li></ul>" );
assertTrue( "heading", findNoCase( "## Title", md ) > 0 );
assertTrue( "bold", findNoCase( "**bold**", md ) > 0 );
assertTrue( "link", findNoCase( "[a link](https://x.test)", md ) > 0 );
assertTrue( "list", findNoCase( "- one", md ) > 0 );
assert( "named argument is BoxLang's name", trim( htmlToMarkdown( html = "<p><em>x</em></p>" ) ), "*x*" );
assertFalse( "script is dropped", findNoCase( "alert", htmlToMarkdown( "<p>a</p><script>alert(1)</script>" ) ) > 0 );
tbl = htmlToMarkdown( "<table><tr><th>A</th><th style='text-align:right'>B</th></tr><tr><td>1</td><td>2</td></tr></table>" );
assertTrue( "a table with alignment", findNoCase( "| A | B |", tbl ) > 0 && findNoCase( "--:", tbl ) > 0 );
pre = htmlToMarkdown( '<pre><code class="language-cfml">x = 1;</code></pre>' );
assertTrue( "a code block keeps its language", findNoCase( "```cfml", pre ) > 0 );
// The HTML markdown() writes comes back to the same document.
src = "## Title" & nl & nl & "Text with **bold** and `code`." & nl & nl & "- a" & nl & "- b" & nl;
assert( "markdown → HTML → markdown", htmlToMarkdown( markdown( src ) ), MarkdownDocument( src ).toMarkdown() );

// ---- isMarkdownDocument() ---------------------------------------------------
assertTrue( "a document is one", isMarkdownDocument( MarkdownDocument() ) );
assertFalse( "a string is not", isMarkdownDocument( "## x" ) );
assertFalse( "another object is not", isMarkdownDocument( htmlDocument( "<p>x</p>" ) ) );
</cfscript>

<!--- ---- <cfmarkdown>: BoxLang's bx:markdown ---- --->
<cfmarkdown variable="tagHtml">
	# Tag heading

	Body with **bold**.
</cfmarkdown>
<cfset assert( "the tag renders its body into variable", trim( tagHtml ), "<h1>Tag heading</h1>" & chr(10) & "<p>Body with <strong>bold</strong>.</p>" )>

<cfsavecontent variable="tagOut"><cfmarkdown>*out*</cfmarkdown></cfsavecontent>
<cfset assert( "without variable it outputs", trim( tagOut ), "<p><em>out</em></p>" )>

<cfset tagName = "World">
<cfoutput><cfmarkdown variable="tagInterp">Hello **#tagName#**</cfmarkdown></cfoutput>
<cfset assert( "inside cfoutput the body interpolates", trim( tagInterp ), "<p>Hello <strong>World</strong></p>" )>

<cfmarkdown variable="tagUnsafe" unsafe="true"><b>raw</b></cfmarkdown>
<cfset assertTrue( "attributes are options", findNoCase( "<b>raw</b>", tagUnsafe ) GT 0 )>

<cfscript>
suiteEnd();
</cfscript>
