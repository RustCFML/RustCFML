# Markdown

RustCFML renders and edits markdown natively: no module to install, in every build
including the WebAssembly targets. CommonMark 0.31 plus GitHub Flavored Markdown
(tables, strikethrough, task lists, autolinks), with footnotes on request. One parser
([comrak](https://github.com/kivikakk/comrak)) handles everything, so the same text
renders the same way whichever function you use.

| You want to | Use |
|---|---|
| Turn markdown into HTML | `markdown( text )` or `<cfmarkdown>` |
| Turn HTML into markdown | `htmlToMarkdown( html )` |
| Build or edit a document: add sections, move them, fill tables from queries | `MarkdownDocument()` |
| Put a variable into markdown safely | `markdownEscape( value )` |

`markdown()`, `htmlToMarkdown()` and the tag have the same shape as BoxLang's
`bx-markdown` module, so code written for it runs here. One difference matters: see
[Safe by default](#safe-by-default).

---

## Rendering: `markdown()` and `<cfmarkdown>`

```cfml
html = markdown( "Pay **now**, see [the terms](https://acme.example/terms)." );
// <p>Pay <strong>now</strong>, see <a href="https://acme.example/terms">the terms</a>.</p>
```

The tag renders its body. With `variable` it stores the HTML; without, it outputs it:

```cfml
<cfmarkdown variable="intro">
    # Welcome

    This page is written in **markdown**.
</cfmarkdown>
```

The tag removes the indentation every line shares, so markdown indented to match your
template is not read as a code block. Outside `<cfoutput>` a `#` in the body is plain
text, so headings need no escaping. Any other attribute is an [option](#options), for
example `<cfmarkdown unsafe="true">`.

> **`#` in CFML strings.** In a string literal a `#` is written `##`, so
> `markdown( "## Title" )` is the markdown `# Title`, a level-1 heading. A level-2
> heading is `"#### Title"`. The tag body (outside `<cfoutput>`) and the
> `.heading()` method avoid this altogether.

## Safe by default

Markdown often comes from users or agents, so by default:

- **raw HTML is dropped** (`<b>`, `<script>`, `<iframe>`, …);
- **dangerous link targets are blanked**: `javascript:`, `vbscript:`, `file:`, and
  `data:` except images.

```cfml
markdown( "<b>hi</b> [x](javascript:alert(1))" );
// <p><!-- raw HTML omitted -->hi<!-- raw HTML omitted --> <a href="">x</a></p>

markdown( "<b>hi</b>", { unsafe = true } );     // trusted content: pass it through
markdown( "<b>hi</b>", { escapeHtml = true } ); // show the tags as text instead
```

BoxLang's `bx-markdown` passes raw HTML through. To match it application-wide, set
`markdown.unsafe` in `.cfconfig.json` (see [Configuration](#configuration)).

## `htmlToMarkdown()`

```cfml
md = htmlToMarkdown( "<h2>Title</h2><p>Some <strong>bold</strong> text.</p>" );
// ## Title
//
// Some **bold** text.
```

Headings, paragraphs, lists (including task lists), links, images, emphasis, code and
code blocks (the `language-x` class becomes the fence language), blockquotes, tables
(with `align` or `text-align`) and line breaks convert. Elements markdown has no word
for keep their text. `script`, `style` and form controls are dropped.

## `markdownEscape()`: putting data into markdown

A string is markdown, so a value dropped into one is formatted: a product called
`C*` or `__init__` turns into italics or bold. Escape values you don't control:

```cfml
markdown( "Customer: " & markdownEscape( customer.name ) );
```

Inside a `MarkdownDocument`, `{ text = value }` does the same job (see below).

---

## `MarkdownDocument()`: build and edit

A `MarkdownDocument` is a markdown document held as a tree. Methods that change it
return the document, so calls chain. `toHtml()`, `toMarkdown()` and the other
`to…` methods give you the result.

```cfml
doc = MarkdownDocument()
    .heading( "Invoice INV-1042" )
    .paragraph( "Please pay **within 30 days**." )
    .table( data = lines, columnList = "item,amount", align = "left,right" )
    .heading( "Notes", 2 )
    .markdown( agentNotes );          // any markdown, appended as blocks

html = doc.toHtml();
md   = doc.toMarkdown();
```

`MarkdownDocument()` starts empty. `MarkdownDocument( text )` parses markdown,
`MarkdownDocument( struct )` loads the [struct form](#the-struct-form), and
`MarkdownDocument( html = text )` converts HTML.

Each method's first parameter is the one you'd pass on its own, so the short form is
positional: `.heading( "Notes", 2 )`. CFML doesn't allow positional and named arguments
in the same call, so use one style or the other.

### The mixing rule

> **A string is markdown. `{ text = value }` is literal text. A struct with a `type`
> is a tree node. An array is a sequence of any of these.**

What a string is parsed *as* depends on where it goes:

| Where | Strings are | Methods |
|---|---|---|
| **One block**: you're making a specific heading, paragraph, list item or cell | Inline markdown only: `**bold**`, `` `code` ``, `[link](url)`, `~~gone~~` | `.heading()`, `.paragraph()`, list items, table headers and cells, `.quote()`, `.setText()` |
| **Any blocks**: you're adding content | Full markdown: any number of blocks | `.markdown()`, `insertAt/insertBefore/insertAfter`, `append`, `replace`, `appendToSection`, `children` arrays |

In one block, syntax that would start a different block stays literal:
`.heading( "1. Introduction" )` is a heading, not a numbered list, and
`.paragraph( "# hashtag" )` is a paragraph. A blank line there is an error, because
two paragraphs can't go where one was asked for; use `.markdown()` for that.

### Text runs

Wherever a method takes text it also takes an **array of runs**. A run is a string
(markdown) or a struct:

```cfml
.paragraph( [
    "Balance for ",
    { text = customer.name },                    // literal: never formatted
    ": ",
    { text = "1,240.00", bold = true },
    " — ",
    { link = "https://acme.example/pay", text = "pay online" },
    { text = "", footnote = "Within 30 days of the invoice date." }
] )
```

| Key | Meaning |
|---|---|
| `text` | Literal text. A nested array is itself runs |
| `markdown` | Inline markdown, the same as a bare string |
| `bold` / `strong`, `italic` / `emphasis`, `strike` / `strikethrough` | Formatting |
| `code = true` | Inline code |
| `link = url`, `title` | A link around the text |
| `image = url`, `alt`, `title` | An image |
| `linebreak` / `break` | A hard line break |
| `footnote = "…"` | A footnote after the text (turns footnotes on). `footnote = true` makes the text itself the note |

These are the keys the Typst `Document()` builder takes, so one runs array can feed
both. Typst-only keys (`size`, `color`, `font`, `align`, `underline`, …) are
ignored here. Any other key is an error. Each string in a runs array is parsed on its
own, so `[ "**", x, "**" ]` does not make `x` bold: use `{ text = x, bold = true }`.

### Building blocks

Every method takes `id` (a name for the new block) and `into` (the id of a
blockquote or list item to add it to, instead of the end of the document).

| Method | Parameters, in order |
|---|---|
| `heading` / `h` | `text`, `level = 1` (`depth` also works) |
| `paragraph` / `p` / `text` | `text` |
| `list` | `items`, `ordered = false`, `start = 1`. An item is a string or runs (one paragraph each), or `{ text \| markdown \| run keys, checked, children }` |
| `taskList` | `items`; string items are unchecked |
| `quote` / `blockquote` | `content` (one paragraph); `children` for blocks |
| `code` / `pre` | `text`, `language` (`lang`), `meta` |
| `table` | `data`, `columnList`, `headers`, `align`, `cellFormat`, `columnFormats` (see [Tables](#tables)) |
| `image` | `url`, `alt`, `title` |
| `rule` / `hr` | — |
| `html` | `markup`: a raw HTML block, output only when `unsafe` |
| `markdown` | `source`: any markdown, appended as blocks |

### Tables

`data` is a query, an array of structs, or an array of arrays, as for
`spreadsheetAddRows`. Column names, or the first row of an array of arrays, become the
header unless you give `headers`. `headers` is markdown, because you wrote it.

```cfml
doc.table(
    data          = orders,                      // a query
    columnList    = "sku,name,notes,amount",
    headers       = "SKU,Item,Notes,**Amount**",
    align         = "left,left,left,right",
    columnFormats = {
        notes  = { format = "markdown" },
        amount = { align = "right", numberFormat = "9,999.00" }
    }
);
```

**Data is text, not markdown.** Values from `data` are shown exactly as they are, so a
product called `C*` stays `C*`. To have them parsed as markdown, opt in at one of
three levels. The most specific wins:

| Level | How |
|---|---|
| One cell | Put a run in the data: `{ markdown = value }` or `{ text = value }` |
| One column | `columnFormats = { notes = { format = "markdown" } }` |
| Whole table | `cellFormat = "markdown"` |
| Default | Plain text |

`columnFormats` is keyed by column name, or by 1-based number for an array of arrays.
Each entry takes `format`, `align`, `numberFormat` and `dateFormat` (masks as for the
CFML functions of those names). This is the same shape as Typst's
`Document().table()`. A newline in a cell becomes a space, because GFM tables are one
line per row.

### Addressing blocks

Edits name a block in one of three ways:

- **a number** is a **1-based position** among the document's top-level blocks (or
  among the children of `parent`, where a method takes one);
- **a string** is a block's **id**;
- failing that, a string is the **text of a heading** (case-insensitive, first match;
  a leading `#` is ignored).

Every block has an id: minted as `b1`, `b2`, …, or the one you gave with `id = "…"`.
An id can't be a number, so a number always means a position. Links and images also
have ids, so you can change a URL in place. Other inline content changes through
`setText()` on its block.

```cfml
doc.insertAfter( 2, "A new **paragraph**." )   // after the 2nd block
   .insertBefore( "Payment terms", tableMarkdown )
   .remove( "b14" );

newIds = doc.lastIds();   // the ids of the blocks the last edit created
```

Positions are handy for one edit. For several edits in a row, prefer ids or headings,
because positions shift as blocks move.

| Method | Effect |
|---|---|
| `insertAt( position, content [, parent] )` | The first new block lands at `position`; `len + 1` appends |
| `insertBefore( target, content )` / `insertAfter( target, content )` | Next to a block |
| `append( parent, content )` / `prepend( parent, content )` | Into a container: a list takes items, a table takes rows (`[ "a", "b" ]` is one row), a blockquote or list item takes blocks, a paragraph takes inline content |
| `replace( target, content )` | Swap one block for any number |
| `remove( target )` | — |
| `move( target, position [, parent] )` | `position` is where the block ends up |
| `moveBefore( target, other )` / `moveAfter( target, other )` | — |
| `set( target, field, value )` / `set( target, { … } )` | `level`, `language`, `meta`, `value`, `ordered`, `start`, `spread`, `checked`, `align`, `url`, `title`, `alt`, `id` |
| `setText( target, content )` | Replace a paragraph's, heading's, cell's, list item's or link's text (or a code block's code) |
| `get( target )` | The block as a [struct](#the-struct-form) (a copy) |
| `find( [type] [, text] [, level] )` | Matching blocks, links and images, in document order. `text` matches part of the plain text, ignoring case |

Each edit applies completely or not at all. If an edit fails (an unknown id, a row
with the wrong number of cells), it throws and the document is unchanged.

### Sections

A section is a heading plus everything after it, up to the next heading at the same or
a higher level. That's usually the unit people want to move.

| Method | Effect |
|---|---|
| `outline()` | `[ { id, level, text, position } ]` for every heading: read this first |
| `section( heading )` | `{ id, level, text, position, ids }` |
| `appendToSection( heading, content )` | At the very end of the section, after its subsections |
| `replaceSection( heading, content [, keepHeading = true] )` | — |
| `removeSection( heading )` | The heading and its body |
| `moveSection( heading, position )` | — |
| `moveSectionBefore( heading, other )` / `moveSectionAfter( heading, other )` | After a heading means after that heading's whole section |

```cfml
doc.appendToSection( "Payment terms", "Bank transfer only." )
   .moveSection( "Notes", 1 );
```

### Results

| Method | Returns |
|---|---|
| `toHtml( [options] )` | HTML |
| `toMarkdown( [options] )` (or `toString()`) | Markdown |
| `toStruct()` / `toJSON()` | The [struct form](#the-struct-form) / its JSON |
| `toText()` | The text with no markup |
| `stats()` | `{ chars, words, lines, blocks, headings, paragraphs, lists, codeBlocks, tables, links, images, footnotes }` |
| `frontMatter()` | The front matter, parsed as YAML into a struct |
| `setFrontMatter( struct \| string )` | Sets it (chainable) |
| `len()` | Number of top-level blocks |
| `copy()` | An independent copy. `duplicate( doc )` does the same |

A `MarkdownDocument` is shared by reference, like a struct: `b = a` gives two names for
one document. Another document can be content too:
`doc.insertAfter( 2, otherDoc )` copies its blocks in.

### Front matter

A leading `---` block is front matter. It stays out of the body and is written back
unchanged by `toMarkdown()`:

```cfml
page = MarkdownDocument( fileRead( "post.md" ) );
title = page.frontMatter().title;
page.setFrontMatter( { title = "New title", draft = false } );
```

---

## The struct form

`toStruct()` returns the document as plain CFML data, and `MarkdownDocument( struct )`
loads it. That makes a document something an agent, or an MCP tool, can read and write
as JSON without parsing markdown. The node shape follows
[mdast](https://github.com/syntax-tree/mdast).

```json
{
  "type": "document",
  "profile": ["commonmark", "gfm"],
  "children": [
    { "id": "b1", "type": "heading", "depth": 1, "children": [
      { "type": "text", "value": "Invoice INV-1042" } ] },
    { "id": "b2", "type": "paragraph", "children": [
      { "type": "text", "value": "Please pay " },
      { "type": "strong", "children": [ { "type": "text", "value": "now" } ] } ] }
  ],
  "definitions": []
}
```

| Node | Fields | Children |
|---|---|---|
| `paragraph` | — | inline |
| `heading` | `depth` 1–6 | inline |
| `thematicBreak` | — | — |
| `code` | `lang`, `meta`, `value` | — |
| `html` | `value` | — |
| `blockquote` | — | blocks |
| `list` | `ordered`, `start`, `spread`, `marker` | `listItem` |
| `listItem` | `checked` (task lists) | blocks |
| `table` | `align` (one entry per column: `left`, `right`, `center` or null) | `tableRow`; the first row is the header |
| `tableRow` | — | `tableCell` |
| `tableCell` | — | inline |
| `text`, `inlineCode` | `value` | — |
| `emphasis`, `strong`, `delete` | — | inline |
| `link` | `url`, `title` | inline |
| `image` | `url`, `alt`, `title` | — |
| `softBreak`, `hardBreak` | — | — |
| `footnoteReference` | `identifier` | — |
| `footnoteDefinition` | `identifier` (lives in `definitions`) | blocks |

Loading is forgiving where that's safe and strict where it isn't:

- **Strings are allowed anywhere in a `children` array.** They are parsed by slot, so
  `{ "type": "listItem", "children": [ "Ship **v2**" ] }` works. `toStruct()` always
  returns full nodes.
- Keys and `type` values are case-insensitive (Lucee uppercases unquoted struct-literal
  keys). Output is camelCase. A `null` field is treated as absent.
- Builder names are accepted as aliases (`level`, `language`, `href`, `src`).
- **Unknown fields are kept** and handed back by `toStruct()`, so a newer writer's
  fields survive a round trip.
- **Rejected:** an unknown `type`; a node in the wrong place (a paragraph directly in a
  list); a heading depth outside 1–6; a duplicate, empty or numeric id; a link or image
  without a `url`; a table row with the wrong number of cells; a line break in a table
  cell, or a hard break in a heading; a GFM node when `profile` has no `gfm`; a
  footnote when it has no `footnotes`.

---

## Guarantees and limits

- **Lossless.** A document parsed from markdown renders exactly the HTML `markdown()`
  gives for the same text. This is checked against every CommonMark 0.31.2 spec
  example and every cmark-gfm extension example.
- **Round trip.** `MarkdownDocument( doc.toMarkdown() )` is the same document
  (apart from ids). The markdown text is not byte-for-byte the original:
  `toMarkdown()` picks its own markers (`-` bullets, ``` fences, `#` headings), and
  writes `<!-- end list -->` between two adjacent lists so they stay separate.
- **Reference links are resolved.** `[x][ref]` becomes a link with its URL, and is
  written back as `[x](url)`. Link reference definitions are not kept.
- Bold directly inside bold (`****x****`) is written back as plain bold. It looks the
  same.
- Dialect features outside CommonMark and GFM (math, admonitions, GitHub alerts, wiki
  links, attribute blocks, definition lists) are not supported. They read as ordinary
  text.
- `writeDump( doc )` shows a native-object box. Use `doc.outline()` or `doc.toStruct()`
  to look inside.

---

## Options

`markdown()`, `htmlToMarkdown()`, `MarkdownDocument()`, `toHtml()` and `toMarkdown()`
take an optional options struct. Keys are case-insensitive; an unknown key is an error.

| Key | Default | Effect |
|---|---|---|
| `gfm` | `true` | Tables, strikethrough, task lists, autolinks |
| `footnotes` | `false` | `[^1]` footnotes |
| `frontMatter` | `true` | Treat a leading `---` block as front matter |
| `unsafe` | `false` | Output raw HTML and every link target |
| `escapeHtml` | `false` | With `unsafe` off, show raw HTML as text instead of dropping it |
| `anchors` | `false` | Give headings `id`s and anchor links (BoxLang `anchorLinks`) |
| `hardBreaks` | `false` | Every newline is a `<br>` |
| `tableClass` | `""` | `class` on `<table>` (BoxLang uses `"table"`) |
| `dedent` | `false` | Remove shared indentation first (the tag always does) |
| `width` | `0` | `toMarkdown()`: wrap lines at this width; 0 = don't wrap |
| `listMarker` | `"-"` | `toMarkdown()`: bullet character, `-`, `*` or `+` |

## Configuration

Application-wide defaults go in `.cfconfig.json`. Per-call options override them.

```json
{
  "markdown": {
    "unsafe": false,
    "anchors": true,
    "tableClass": "table",
    "footnotes": true
  }
}
```

Accepted keys: `gfm`, `footnotes`, `frontMatter`, `unsafe`, `escapeHtml`, `anchors`,
`hardBreaks`, `tableClass`. To render like BoxLang's `bx-markdown` defaults, set
`"unsafe": true, "anchors": true, "tableClass": "table"`.
