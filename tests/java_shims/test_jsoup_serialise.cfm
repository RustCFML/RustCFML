<cfscript>
suiteBegin("java shim: org.jsoup serialises as jsoup 1.10.1 does (GH ##517)");

// Every expected string below was captured from the real jsoup 1.10.1 jar (the
// one Preside ships), not written by hand. Callers compare jsoup's output
// exactly — Preside's EmailStyleInliner and click-tracking tests among them —
// so the bytes matter: pretty-printing, trailing spaces and all.

nl = chr(10); tb = chr(9);
jsoup = CreateObject( "java", "org.jsoup.Jsoup" );

// ---- the issue's repro ------------------------------------------------------
src = "<!DOCTYPE html><html><head><title>T</title></head><body><p>a" & nl & "b</p></body></html>";
doc = jsoup.parse( src );
assert( "toString() pretty-prints, one space per level, doctype lower-cased"
      , doc.toString()
      , "<!doctype html>" & nl & "<html>" & nl & " <head>" & nl & "  <title>T</title>" & nl & " </head>" & nl & " <body>" & nl & "  <p>a b</p>" & nl & " </body>" & nl & "</html>" );
assert( "Document.html() is the same as toString()", doc.html(), doc.toString() );
assert( "Element.html() is trimmed inner HTML with whitespace normalised", doc.body().html(), "<p>a b</p>" );
assert( "Element.outerHtml() includes the element"
      , doc.body().outerHtml(), "<body>" & nl & " <p>a b</p>" & nl & "</body>" );

doc.outputSettings().prettyPrint( false );
assert( "prettyPrint( false ) writes the tree as parsed, doctype still lower-case"
      , doc.toString(), "<!doctype html><html><head><title>T</title></head><body><p>a" & nl & "b</p></body></html>" );
assert( "and Element.html() keeps the newline", doc.body().html(), "<p>a" & nl & "b</p>" );

doc4 = jsoup.parse( src );
doc4.outputSettings().indentAmount( 4 );
assert( "indentAmount( 4 )"
      , doc4.toString()
      , "<!doctype html>" & nl & "<html>" & nl & "    <head>" & nl & "        <title>T</title>" & nl & "    </head>" & nl & "    <body>" & nl & "        <p>a b</p>" & nl & "    </body>" & nl & "</html>" );

// ---- OutputSettings read back ------------------------------------------------
s = jsoup.parse( "<p>x</p>" ).outputSettings();
assertTrue( "prettyPrint() defaults to true", s.prettyPrint() );
assert( "indentAmount() defaults to 1", s.indentAmount(), 1 );
assert( "charset() defaults to UTF-8", s.charset(), "UTF-8" );
s.charset( "ASCII" );
assert( "charset( 'ASCII' ) is Java's US-ASCII", s.charset(), "US-ASCII" );
assertThrows( "an unknown charset throws, as Charset.forName does", function(){ s.charset( "no-such-charset" ); } );
assertThrows( "a negative indent is refused", function(){ s.indentAmount( -1 ); } );

// ---- whitespace-only text between blocks -----------------------------------
// jsoup keeps each whitespace run as its own text node and writes it as one
// space — hence the trailing spaces. The newline before </body> and the one
// after it are two nodes, so that line ends in TWO spaces.
wsSrc = "<html>" & nl & "<head>" & nl & tb & "<title>  My   email  </title>" & nl & "</head>" & nl & "<body>" & nl & "<p>Lorem   ipsum" & nl & "dolor</p>" & nl & "<p>second</p>" & nl & "</body>" & nl & "</html>";
assert( "whitespace text nodes print as single spaces; <title> preserves its own"
      , jsoup.parse( wsSrc ).toString()
      , "<html>" & nl & " <head> " & nl & "  <title>  My   email  </title> " & nl & " </head> " & nl & " <body> " & nl & "  <p>Lorem ipsum dolor</p> " & nl & "  <p>second</p>  " & nl & " </body>" & nl & "</html>" );

// ---- inline / block / text interplay ------------------------------------------
assert( "text after a block is not indented; an inline element after text is"
      , jsoup.parse( "<div><p>para</p>trailing text <b>bold</b><p>p2</p> more</div>" ).body().html()
      , "<div>" & nl & " <p>para</p>trailing text " & nl & " <b>bold</b>" & nl & " <p>p2</p> more" & nl & "</div>" );
assert( "<pre> keeps its whitespace"
      , jsoup.parse( "<div><pre>  line1" & nl & "    line2  <b> bold  x </b>" & nl & "</pre><p>after</p></div>" ).body().html()
      , "<div>" & nl & " <pre>  line1" & nl & "    line2  <b> bold  x </b>" & nl & "</pre>" & nl & " <p>after</p>" & nl & "</div>" );
assert( "comments are always indented when pretty-printing"
      , jsoup.parse( "<!-- top --><html><head><!-- in head --></head><body><!-- first --><p>a<!-- mid -->b</p><div><!-- in div --></div></body></html>" ).toString()
      , "<!-- top -->" & nl & "<html>" & nl & " <head>" & nl & "  <!-- in head -->" & nl & " </head>" & nl & " <body>" & nl & "  <!-- first -->" & nl & "  <p>a" & nl & "   <!-- mid -->b</p>" & nl & "  <div>" & nl & "   <!-- in div -->" & nl & "  </div>" & nl & " </body>" & nl & "</html>" );
assert( "SVG names lower-cased; an unknown self-closing tag writes back as <x />"
      , jsoup.parse( "<div><svg width='10' height='10' viewBox='0 0 10 10'><circle cx='5' cy='5' r='4'/></svg></div>" ).body().html()
      , "<div>" & nl & " <svg width=""10"" height=""10"" viewbox=""0 0 10 10"">" & nl & "  <circle cx=""5"" cy=""5"" r=""4"" />" & nl & " </svg>" & nl & "</div>" );
assert( "<div/> is closed at once, as jsoup's tree builder does"
      , jsoup.parse( "<div/>x<my-widget/><p>y</p>" ).body().html()
      , "<div></div>x" & nl & "<my-widget />" & nl & "<p>y</p>" );

// ---- escaping and attributes ------------------------------------------------
assert( "entities: & < > escaped in text, "" and & in attributes, nbsp named"
      , jsoup.parse( "<p>a &amp; b &lt; c &gt; d &quot;q&quot; &nbsp;x&nbsp; &copy; &eacute;</p><a title='a &quot;q&quot; &lt;t&gt; &amp;' href='?a=1&amp;b=2'>l</a>" ).body().html()
      , "<p>a &amp; b &lt; c &gt; d ""q"" &nbsp;x&nbsp; " & chr(169) & " " & chr(233) & "</p>" & nl & "<a title=""a &quot;q&quot; <t> &amp;"" href=""?a=1&amp;b=2"">l</a>" );
assert( "boolean attributes collapse; a valueless attribute writes back bare"
      , jsoup.parse( "<input type='checkbox' checked><input disabled='disabled'><input readonly=''><option selected='yes'>x</option><video autoplay controls muted></video><script async src='x.js'></script>" ).body().html()
      , "<input type=""checkbox"" checked>" & nl & "<input disabled>" & nl & "<input readonly>" & nl & "<option selected=""yes"">x</option>" & nl & "<video autoplay controls muted></video>" & nl & "<script async src=""x.js""></script>" );
vdoc = jsoup.parse( "<video controls autoplay></video>" );
vdoc.select( "video" )[ 1 ].attr( "controls", "" );
assert( "setting a valueless attribute makes it an ordinary one"
      , vdoc.body().html(), "<video controls="""" autoplay></video>" );

// ---- charset( "ASCII" ): Preside's emailStyleInlinerAscii -------------------
adoc = jsoup.parse( "<html><body><p>Caf" & chr(233) & " na" & chr(239) & "ve " & chr(8364) & "5 " & chr(8212) & " " & chr(8220) & "quoted" & chr(8221) & " " & chr(26085) & chr(26412) & " " & chr(128512) & " " & chr(160) & "</p><img alt='" & chr(233) & "t" & chr(233) & " " & chr(8364) & "' src='a.png'></body></html>" );
adoc.outputSettings().charset( "ASCII" );
assert( "non-ASCII becomes a named entity where jsoup's base table has one, else &##x...;"
      , adoc.html()
      , "<html>" & nl & " <head></head>" & nl & " <body>" & nl & "  <p>Caf&eacute; na&iuml;ve &##x20ac;5 &##x2014; &##x201c;quoted&##x201d; &##x65e5;&##x672c; &##x1f600; &nbsp;</p>" & nl & "  <img alt=""&eacute;t&eacute; &##x20ac;"" src=""a.png"">" & nl & " </body>" & nl & "</html>" );

// ---- outline mode -----------------------------------------------------------
odoc = jsoup.parse( "<div><p>a</p><span>b</span></div>" );
odoc.outputSettings().outline( true );
assert( "outline( true ) formats every element as a block"
      , odoc.body().html(), "<div>" & nl & " <p>a</p>" & nl & " <span>b</span>" & nl & "</div>" );

// ---- parse always builds a document -----------------------------------------
assert( "parseBodyFragment builds the document shell around the fragment"
      , jsoup.parseBodyFragment( "<p class='x'>frag</p>" ).toString()
      , "<html>" & nl & " <head></head>" & nl & " <body>" & nl & "  <p class=""x"">frag</p>" & nl & " </body>" & nl & "</html>" );
assert( "and an orphan <td> keeps only its text, as in jsoup"
      , jsoup.parseBodyFragment( "<td class=""cell"">only me</td>" ).toString()
      , "<html>" & nl & " <head></head>" & nl & " <body>" & nl & "  only me" & nl & " </body>" & nl & "</html>" );
assert( "Jsoup.parse of bare text is a whole document too"
      , jsoup.parse( "a string without tags" ).toString()
      , "<html>" & nl & " <head></head>" & nl & " <body>" & nl & "  a string without tags" & nl & " </body>" & nl & "</html>" );

suiteEnd();
</cfscript>
