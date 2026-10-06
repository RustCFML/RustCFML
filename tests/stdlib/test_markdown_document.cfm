<cfscript>
suiteBegin("MarkdownDocument(): an editable markdown tree");

// A `#` in a CFML string literal is doubled: "## x" is the markdown "# x" and
// "#### x" is "## x". (One of the reasons the .heading() verb exists.)
nl = chr(10);

// ---- building -----------------------------------------------------------------
// Builder verbs append and return the document, so calls chain. Their first
// parameter is the one you'd pass alone, so the short form is positional.
doc = MarkdownDocument()
	.heading( "Invoice INV-1042" )
	.paragraph( "Please pay **now**." )
	.heading( "Lines", 2 )
	.list( [ "Labour", "Parts" ] );
assert( "builds markdown", doc.toMarkdown(),
	"## Invoice INV-1042" & nl & nl & "Please pay **now**." & nl & nl & "#### Lines" & nl & nl & "- Labour" & nl & "- Parts" & nl );
assert( "and HTML", trim( doc.toHtml() ),
	"<h1>Invoice INV-1042</h1>" & nl & "<p>Please pay <strong>now</strong>.</p>" & nl & "<h2>Lines</h2>" & nl & "<ul>" & nl & "<li>Labour</li>" & nl & "<li>Parts</li>" & nl & "</ul>" );
assert( "len counts top-level blocks", doc.len(), 4 );
assert( "named arguments work", MarkdownDocument().heading( text = "x", level = 3 ).toMarkdown(), "###### x" & nl );
assert( "depth is accepted for level", MarkdownDocument().heading( text = "x", depth = 2 ).toMarkdown(), "#### x" & nl );
assertThrows( "a level outside 1-6 is an error", function(){ MarkdownDocument().heading( "x", 7 ); } );

// ---- the mixing rule: a string is markdown, { text = } is literal -----------
// In a single block (heading, paragraph, list item, cell) a string is INLINE
// markdown: block syntax at the start of a line stays literal.
assert( "a numbered heading stays a heading", trim( MarkdownDocument().heading( "1. Introduction" ).toHtml() ), "<h1>1. Introduction</h1>" );
assert( "a hash in a paragraph stays text", trim( MarkdownDocument().paragraph( "## tag" ).toHtml() ), "<p>## tag</p>" );
// Runs: the same shape the Typst Document() builder takes.
p = MarkdownDocument().paragraph( [ "Balance for ", { text = "C*_x_" }, ": ", { text = "42.00", bold = true }, " ", { link = "https://x.test", text = "pay" } ] );
assert( "runs mix markdown and literal text", trim( p.toHtml() ), '<p>Balance for C*_x_: <strong>42.00</strong> <a href="https://x.test">pay</a></p>' );
assert( "italic, strike and code runs", trim( MarkdownDocument().paragraph( [ { text = "a", italic = true }, { text = "b", strike = true }, { text = "c", code = true } ] ).toHtml() ),
	"<p><em>a</em><del>b</del><code>c</code></p>" );
assert( "a markdown run", trim( MarkdownDocument().paragraph( [ { markdown = "*x*" } ] ).toHtml() ), "<p><em>x</em></p>" );
assert( "a line break run", trim( MarkdownDocument().paragraph( [ "a", { linebreak = true }, "b" ] ).toHtml() ), "<p>a<br />" & nl & "b</p>" );
assert( "an image run", trim( MarkdownDocument().paragraph( [ { image = "a.png", alt = "A" } ] ).toHtml() ), '<p><img src="a.png" alt="A" /></p>' );
// Typst-only presentation keys are ignored, so one runs array feeds both builders.
assert( "presentation keys are ignored", trim( MarkdownDocument().paragraph( [ { text = "x", color = "##f00", size = 12 } ] ).toHtml() ), "<p>x</p>" );
assertThrows( "an unknown run key is an error", function(){ MarkdownDocument().paragraph( [ { text = "x", wobble = true } ] ); } );
assertThrows( "a blank line can't go in one paragraph", function(){ MarkdownDocument().paragraph( "a" & nl & nl & "b" ); } );
// A footnote run creates the reference and the definition, and turns footnotes on.
fn = MarkdownDocument().paragraph( [ "Due in 30 days", { footnote = "From the invoice date." } ] );
assertTrue( "a footnote run", findNoCase( "From the invoice date.", fn.toHtml() ) > 0 && findNoCase( "footnote", fn.toHtml() ) > 0 );
assertTrue( "and it round-trips", findNoCase( "[^1]:", fn.toMarkdown() ) > 0 && MarkdownDocument( fn.toMarkdown(), { footnotes = true } ).toHtml() == fn.toHtml() );

// In a block slot (.markdown(), the insert methods, children) a string is
// full markdown: any number of blocks.
m = MarkdownDocument().markdown( "## A" & nl & nl & "Para." & nl & nl & "| x | y |" & nl & "|---|---|" & nl & "| 1 | 2 |" );
assert( "markdown() appends parsed blocks", m.len(), 3 );
assert( "the third is a table", m.get( 3 ).type, "table" );

// ---- lists ----------------------------------------------------------------------
l = MarkdownDocument().list( items = [ "one", "two" ], ordered = true, start = 5 );
assert( "an ordered list", l.toMarkdown(), "5. one" & nl & "6. two" & nl );
t = MarkdownDocument().taskList( [ "todo", { text = "done", checked = true } ] );
assert( "a task list", t.toMarkdown(), "- [ ] todo" & nl & "- [x] done" & nl );
nested = MarkdownDocument().list( [ "one", { text = "two", children = [ { type = "list", ordered = true, children = [ "a", "b" ] } ] } ] );
assertTrue( "a nested list", findNoCase( "<ol>", nested.toHtml() ) > 0 );
// A list item string is one paragraph; `1. x` in it stays text.
assert( "a list item string is inline", trim( MarkdownDocument().list( [ "1. first" ] ).toHtml() ), "<ul>" & nl & "<li>1. first</li>" & nl & "</ul>" );

// ---- other verbs ------------------------------------------------------------------
assert( "code", MarkdownDocument().code( "x = 1;", "cfml" ).toMarkdown(), "```cfml" & nl & "x = 1;" & nl & "```" & nl );
assert( "code with backticks gets a longer fence", MarkdownDocument().code( "a ``` b" ).toMarkdown(), "````" & nl & "a ``` b" & nl & "````" & nl );
assert( "quote", MarkdownDocument().quote( "Quoted **text**" ).toMarkdown(), "> Quoted **text**" & nl );
assert( "rule", MarkdownDocument().rule().toMarkdown(), "-----" & nl );
assert( "image", trim( MarkdownDocument().image( "a.png", "Alt" ).toHtml() ), '<p><img src="a.png" alt="Alt" /></p>' );
h = MarkdownDocument().html( "<div>x</div>" );
assertFalse( "raw html is dropped by default", findNoCase( "<div>", h.toHtml() ) > 0 );
assertTrue( "and rendered when unsafe", findNoCase( "<div>x</div>", h.toHtml( { unsafe = true } ) ) > 0 );

// ---- tables: data is text unless you say otherwise ------------------------------
q = queryNew( "sku,name,amount", "varchar,varchar,double", [ [ "A1", "C* Pro", 1234.5 ], [ "B2", "__init__", 40 ] ] );
tq = MarkdownDocument().table( q );
assertTrue( "query data is literal", findNoCase( "C* Pro", tq.toHtml() ) > 0 && findNoCase( "__init__", tq.toHtml() ) > 0 );
assertFalse( "not emphasis", findNoCase( "<em>", tq.toHtml() ) > 0 || findNoCase( "<strong>", tq.toHtml() ) > 0 );
assertTrue( "column names are the header", findNoCase( "<th>sku</th>", tq.toHtml() ) > 0 );
tf = MarkdownDocument().table(
	data          = q,
	columnList    = "name,amount",
	headers       = "Item,**Amount**",
	align         = "left,right",
	columnFormats = { amount = { numberFormat = "9,999.00" } }
);
html = tf.toHtml();
assertTrue( "columnList picks columns", findNoCase( "A1", html ) == 0 );
assertTrue( "headers are markdown", findNoCase( "<strong>Amount</strong>", html ) > 0 );
assertTrue( "numberFormat", findNoCase( "1,234.50", html ) > 0 && findNoCase( "40.00", html ) > 0 );
assertTrue( "align", findNoCase( '<td align="right">1,234.50</td>', html ) > 0 );
// Markdown opt-in: whole table, then per column overrides, then per cell.
notes = [ { name = "**bold**", note = "*em*" } ];
assertTrue( "cellFormat markdown parses every cell", findNoCase( "<strong>bold</strong>", MarkdownDocument().table( data = notes, cellFormat = "markdown" ).toHtml() ) > 0 );
mixed = MarkdownDocument().table( data = notes, cellFormat = "markdown", columnFormats = { name = { format = "text" } } ).toHtml();
assertTrue( "a column can opt back out", findNoCase( "**bold**", mixed ) > 0 && findNoCase( "<em>em</em>", mixed ) > 0 );
assertTrue( "a column can opt in", findNoCase( "<em>em</em>", MarkdownDocument().table( data = notes, columnFormats = { note = { format = "markdown" } } ).toHtml() ) > 0 );
assertTrue( "a cell run overrides", findNoCase( "<em>x</em>", MarkdownDocument().table( data = [ [ "h" ], [ { markdown = "*x*" } ] ] ).toHtml() ) > 0 );
aa = MarkdownDocument().table( [ [ "A", "B" ], [ 1, 2 ] ] );
assert( "array of arrays: the first row is the header", aa.toMarkdown(), "| A | B |" & nl & "| --- | --- |" & nl & "| 1 | 2 |" & nl );
assert( "columnFormats by number for arrays", trim( MarkdownDocument().table( data = [ [ "A" ], [ 3 ] ], columnFormats = { 1 = { numberFormat = "0.00" } } ).toMarkdown() ), "| A |" & nl & "| --- |" & nl & "| 3.00 |" );
assertTrue( "a pipe in data is escaped", findNoCase( "a \| b", MarkdownDocument().table( [ [ "h" ], [ "a | b" ] ] ).toMarkdown() ) > 0 );
assertThrows( "an unknown column in columnFormats", function(){ MarkdownDocument().table( data = q, columnFormats = { nope = { align = "left" } } ); } );
assertThrows( "headers must match the columns", function(){ MarkdownDocument().table( data = q, headers = "a,b" ); } );

// ---- addressing: a number is a position, a string is an id or heading text ----
d = MarkdownDocument( "## Intro" & nl & nl & "Hello." & nl & nl & "## Payment" & nl & nl & "Pay now." & nl & nl & "## Notes" & nl & nl & "None." );
assert( "six blocks", d.len(), 6 );
d.insertAfter( 2, "A **new** paragraph." );
assert( "insertAfter a position", d.get( 3 ).children[ 1 ].value, "A " );
d.insertAt( 1, "Top." );
assert( "insertAt", trim( d.get( 1 ).children[ 1 ].value ), "Top." );
d.insertBefore( "Payment", "Before payment." );
idx = 0;
outline = d.outline();
assert( "outline lists headings", arrayLen( outline ), 3 );
assert( "outline gives positions", outline[ 2 ].text, "Payment" );
assert( "and the position moved", outline[ 2 ].position, 6 );
// Every block has an id; numeric ids are refused so a number always means a position.
firstId = d.get( 1 ).id;
assertTrue( "ids are minted", len( firstId ) > 0 );
assert( "get by id", d.get( firstId ).type, "paragraph" );
assertThrows( "a numeric id is refused", function(){ MarkdownDocument().paragraph( text = "x", id = "12" ); } );
assertThrows( "an out of range position", function(){ d.get( 99 ); } );
assertThrows( "an unknown id", function(){ d.remove( "nope" ); } );
// lastIds() names what the last edit created, even from a markdown string.
d.appendToSection( "Payment", "Bank transfer." & nl & nl & "Or card." );
assert( "lastIds after a string insert", arrayLen( d.lastIds() ), 2 );
added = d.lastIds()[ 2 ];
assert( "and they resolve", d.get( added ).type, "paragraph" );
d.remove( added );
assertThrows( "remove removes", function(){ d.get( added ); } );

// ---- sections ----------------------------------------------------------------------
s = MarkdownDocument( "## A" & nl & nl & "a1" & nl & nl & "#### A.1" & nl & nl & "a2" & nl & nl & "## B" & nl & nl & "b1" );
sec = s.section( "A" );
assert( "a section runs to the next heading of its level", arrayLen( sec.ids ), 4 );
s.moveSection( "B", 1 );
assert( "moveSection to the top", s.outline()[ 1 ].text, "B" );
s.moveSectionAfter( "B", "A" );
assert( "moveSectionAfter a heading goes after its whole section", s.outline()[ 3 ].text, "B" );
s.replaceSection( "A", "Replaced." );
assert( "replaceSection keeps the heading", s.section( "A" ).ids.len(), 2 );
s.removeSection( "A" );
assert( "removeSection", arrayLen( s.outline() ), 1 );
assertThrows( "a paragraph has no section", function(){ s.section( 2 ); } );

// ---- moving and editing ------------------------------------------------------------
mv = MarkdownDocument().paragraph( "one" ).paragraph( "two" ).paragraph( "three" );
mv.move( 3, 1 );
assert( "move to a final position", mv.toText(), "three" & nl & nl & "one" & nl & nl & "two" );
mv.moveAfter( 1, 3 );
assert( "moveAfter", mv.toText(), "one" & nl & nl & "two" & nl & nl & "three" );
mv.setText( 1, "**ONE**" );
assert( "setText", mv.toMarkdown(), "**ONE**" & nl & nl & "two" & nl & nl & "three" & nl );
mv.replace( 2, "## Two" & nl & nl & "2a" );
assert( "replace with several blocks", mv.len(), 4 );
hd = MarkdownDocument().heading( "x" );
hd.set( 1, "level", 3 );
assert( "set a field", hd.toMarkdown(), "###### x" & nl );
lk = MarkdownDocument().paragraph( "[a](https://old.test)" );
linkId = lk.find( "link" )[ 1 ].id;
lk.set( linkId, "url", "https://new.test" );
assertTrue( "links have ids, so their url can be set", findNoCase( "new.test", lk.toMarkdown() ) > 0 );
assertThrows( "an unknown field", function(){ hd.set( 1, "colour", "red" ); } );
// Into a container: list items, table rows, blockquotes.
li = MarkdownDocument().list( items = [ "a" ], id = "theList" );
li.append( "theList", [ "b", "c" ] );
assert( "append items to a list", li.toMarkdown(), "- a" & nl & "- b" & nl & "- c" & nl );
tb = MarkdownDocument().table( data = [ [ "h1", "h2" ], [ 1, 2 ] ], id = "t" );
tb.append( "t", [ "3", "4" ] );
assertTrue( "append a row to a table", findNoCase( "| 3 | 4 |", tb.toMarkdown() ) > 0 );
assertThrows( "a ragged row is refused", function(){ tb.append( "t", [ "only one" ] ); } );
qq = MarkdownDocument().quote( content = "q", id = "q" );
qq.paragraph( text = "inside", into = "q" );
assert( "into puts a verb's block inside a container", qq.toMarkdown(), "> q" & nl & "> " & nl & "> inside" & nl );
// A failed edit leaves the document exactly as it was.
before = tb.toMarkdown();
try { tb.append( "t", [ "x" ] ); } catch ( any e ) {}
assert( "a failed edit changes nothing", tb.toMarkdown(), before );
// …including what loading the content did on the way: a footnote run in a
// rejected row must not leave its definition behind or turn footnotes on.
try { tb.append( "t", [ [ { text = "x", footnote = "orphan" } ] ] ); } catch ( any e ) {}
assert( "a failed edit leaves no footnote behind", arrayLen( tb.toStruct().definitions ), 0 );
assertFalse( "nor turns footnotes on", arrayFind( tb.toStruct().profile, "footnotes" ) > 0 );
// Tables stay rectangular through every edit.
cellId = tb.get( "t" ).children[ 1 ].children[ 1 ].id;
assertThrows( "a single cell can't be removed", function(){ tb.remove( cellId ); } );
assertThrows( "a cell can't be replaced by blocks", function(){ tb.replace( cellId, "x" ); } );
tb.setText( cellId, "**H1**" );
assertTrue( "setText changes a cell", findNoCase( "**H1**", tb.toMarkdown() ) > 0 );
one = MarkdownDocument().table( data = [ [ "only" ] ], id = "solo" );
assertThrows( "a table's last row can't be removed", function(){ one.remove( one.get( "solo" ).children[ 1 ].id ); } );
// Building a large document a block at a time stays linear.
big = MarkdownDocument();
t0 = getTickCount();
for ( i = 1; i <= 2000; i++ ) { big.paragraph( "Paragraph " & i ); }
assert( "2,000 appends", big.len(), 2000 );
assertTrue( "in well under a second", getTickCount() - t0 < 1000 );

// ---- find, stats, text ---------------------------------------------------------------
fd = MarkdownDocument( "## One" & nl & nl & "#### Two" & nl & nl & "text with a [link](https://a.test)" );
assert( "find by type", arrayLen( fd.find( "heading" ) ), 2 );
assert( "find by level", fd.find( type = "heading", level = 2 )[ 1 ].children[ 1 ].value, "Two" );
assert( "find by text", arrayLen( fd.find( text = "with a" ) ), 1 );
st = fd.stats();
assert( "stats headings", st.headings, 2 );
assert( "stats links", st.links, 1 );
assert( "stats words", st.words, 6 );

// ---- the struct form: toStruct() / MarkdownDocument( struct ) --------------------------
tree = MarkdownDocument().heading( "T" ).paragraph( "x **y**" ).toStruct();
assert( "toStruct type", tree.type, "document" );
assert( "children are nodes", tree.children[ 1 ].type, "heading" );
assert( "mdast field names", tree.children[ 1 ].depth, 1 );
// An agent can write nodes with markdown strings inside, in any case.
loaded = MarkdownDocument( {
	TYPE = "document",
	CHILDREN = [
		{ TYPE = "HEADING", DEPTH = 2, CHILDREN = [ "Shipped **v2**" ] },
		"A paragraph of *markdown*.",
		{ type = "list", children = [ "one", { type = "listItem", checked = true, children = [ "done" ] } ] }
	]
} );
assert( "case-insensitive keys and types, strings inside", loaded.toMarkdown(),
	"#### Shipped **v2**" & nl & nl & "A paragraph of *markdown*." & nl & nl & "- one" & nl & "- [x] done" & nl );
reloaded = MarkdownDocument( loaded.toStruct() );
assert( "struct round trip", reloaded.toMarkdown(), loaded.toMarkdown() );
assert( "ids survive it", reloaded.get( 1 ).id, loaded.get( 1 ).id );
custom = MarkdownDocument( { children = [ { type = "paragraph", id = "intro", future = "kept", children = [ "x" ] } ] } );
assert( "a supplied id is kept", custom.get( "intro" ).type, "paragraph" );
assert( "unknown fields are preserved", custom.toStruct().children[ 1 ].future, "kept" );
fromJson = MarkdownDocument( deserializeJSON( custom.toJSON() ) );
assert( "toJSON round trips", fromJson.toMarkdown(), custom.toMarkdown() );
assertThrows( "an unknown type", function(){ MarkdownDocument( { children = [ { type = "bogus" } ] } ); } );
assertThrows( "a list holding a paragraph", function(){ MarkdownDocument( { children = [ { type = "list", children = [ { type = "paragraph" } ] } ] } ); } );
assertThrows( "a duplicate id", function(){ MarkdownDocument( { children = [ { type = "paragraph", id = "a" }, { type = "paragraph", id = "a" } ] } ); } );
assertThrows( "a heading depth of 9", function(){ MarkdownDocument( { children = [ { type = "heading", depth = 9 } ] } ); } );
assertThrows( "a gfm node with gfm off", function(){ MarkdownDocument( { profile = [ "commonmark" ], children = [ { type = "delete", children = [ "x" ] } ] } ); } );

// ---- front matter, copies, other documents ---------------------------------------
fm = MarkdownDocument( "---" & nl & "title: Hello" & nl & "tags: [a, b]" & nl & "---" & nl & nl & "Body." );
assert( "front matter is kept out of the body", fm.len(), 1 );
assert( "frontMatter() parses it", fm.frontMatter().title, "Hello" );
assertTrue( "and toMarkdown() writes it back", left( fm.toMarkdown(), 4 ) == "---" & nl );
fm.setFrontMatter( { title = "Changed" } );
assert( "setFrontMatter from a struct", fm.frontMatter().title, "Changed" );
orig = MarkdownDocument().paragraph( "a" );
cp = orig.copy();
cp.paragraph( "b" );
assert( "copy() is independent", orig.len(), 1 );
dup = duplicate( orig );
dup.paragraph( "c" );
assert( "so is duplicate()", orig.len(), 1 );
host = MarkdownDocument().paragraph( "host" );
host.insertAfter( 1, MarkdownDocument().heading( "Guest" ).paragraph( "g" ) );
assert( "another document's blocks can be inserted", host.len(), 3 );
assertThrows( "but not a document into itself", function(){ host.insertAfter( 1, host ); } );
assert( "MarkdownDocument( html = … )", MarkdownDocument( html = "<h1>T</h1><p><b>x</b></p>" ).toMarkdown(), "## T" & nl & nl & "**x**" & nl );

assertThrows( "an unknown method", function(){ MarkdownDocument().wibble(); } );

suiteEnd();
</cfscript>
