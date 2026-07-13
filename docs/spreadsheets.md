# Spreadsheets

RustCFML has **native, JVM-free spreadsheet support** — read, create, edit and
write real `.xlsx` files with cell values, styling, formulas-safe cell types,
merged cells, freeze panes and column/row sizing. It is a faithful
implementation of the standard CFML `Spreadsheet*` function family (ACF / Lucee
spreadsheet-extension / BoxLang `bx-spreadsheet`), plus a **fluent builder** in
the BoxLang style.

Unlike Adobe ColdFusion, the Lucee spreadsheet extension, and BoxLang's
`bx-spreadsheet` — all of which are Apache POI (Java) under the hood — RustCFML
has no JVM. The engine is backed by the pure-Rust
[`umya-spreadsheet`](https://crates.io/crates/umya-spreadsheet) crate, which
opens an existing workbook into a mutable in-memory model, lets you mutate it,
and writes it back with styling preserved (the POI round-trip model).

> **Native/server capability.** Spreadsheet support is compiled behind the
> `spreadsheet` cargo feature, which is **on by default** for the native CLI and
> server binaries. It is **not** included in the WebAssembly builds (the
> Cloudflare Worker / interactive demo), where the functions report a clear
> "not available in this build" error.

---

## Two ways to work

### 1. The `Spreadsheet*` BIF family (ACF / Lucee compatible)

The workbook is the **first argument** to every function, and mutations are
visible through your variable (the workbook is a shared, reference-typed
object):

```cfml
wb = spreadsheetNew( "Report", true );          // sheet name, xmlformat=true (.xlsx)
spreadsheetSetCellValue( wb, "Hello", 1, 1 );    // value, row, column (1-based)
spreadsheetSetCellValue( wb, 42, 2, 1 );
spreadsheetFormatRow( wb, { bold = true }, 1 );
spreadsheetWrite( wb, expandPath( "/report.xlsx" ), true );  // path, overwrite
```

### 2. The fluent `Spreadsheet()` builder (BoxLang style)

`Spreadsheet()` returns a workbook whose **mutating methods return the workbook
itself**, so calls chain; terminal reads (`getCellValue`, `info`, `readBinary`)
return their value and end the chain:

```cfml
Spreadsheet( "xlsx" )
    .renameSheet( "Results", 1 )
    .addRows( myQuery, 1, 1, true )              // query → rows, with a header row
    .formatRow( { bold = true, bgcolor = "lightblue" }, 1 )
    .addFreezePane( 0, 1 )                        // freeze the header row
    .autoSize()                                   // best-fit every column
    .write( expandPath( "/results.xlsx" ), true );
```

The two styles are interchangeable and operate on the same object — pick
whichever reads better for the task.

---

## Addressing & conventions

- **1-based** rows and columns everywhere (`row 1, column 1` = cell A1), matching
  ACF/Lucee/BoxLang.
- **Styling is a plain struct** — `{ bold: true, bgcolor: "##FFCC00" }`.
- **Colours** accept a name (`red`, `blue`, `lightgray`, …) or a hex string
  (`##RGB`, `##RRGGBB`, or `AARRGGBB`).
- The workbook tracks an **active sheet**; most operations target it until you
  select another with `setActiveSheet` / `setActiveSheetNumber`.
- **Format-method argument order is flexible.** Because the format is always a
  struct and coordinates are always numeric, both the ACF/Lucee ordering
  (`formatRow( wb, {bold:true}, 1 )` — format first) and the BoxLang fluent
  ordering (`.formatCell( 1, 1, {bold:true} )` — coordinates first) work.

---

## Function reference

### Lifecycle
| Function | Signature | Notes |
|---|---|---|
| `SpreadsheetNew` | `(sheetName?, xmlFormat?)` | New workbook. `xmlFormat` true→`.xlsx` (default). |
| `Spreadsheet` | `(typeOrNothing?)` | Fluent-builder entry point; `"xlsx"` (default) or `"xls"`. |
| `SpreadsheetRead` | `(filePath)` | Open an existing `.xlsx` into a mutable workbook (round-trip). |
| `SpreadsheetReadBinary` | `(workbook)` | Serialise the workbook to `.xlsx` bytes (binary). |
| `SpreadsheetWrite` | `(workbook, filePath, overwrite?, password?)` | Write `.xlsx` to disk. |
| `IsSpreadsheetObject` | `(value)` | True if `value` is a workbook. |
| `SpreadsheetInfo` | `(workbook)` | Struct: `sheets`, `sheetnames`, `rowcount`, `columncount`, `format`. |

### Cells, rows & columns
| Function | Signature |
|---|---|
| `SpreadsheetSetCellValue` | `(workbook, value, row, column)` |
| `SpreadsheetGetCellValue` | `(workbook, row, column)` |
| `SpreadsheetAddRow` | `(workbook, data, row?, column=1, insert=true, delimiter=",")` |
| `SpreadsheetAddRows` | `(workbook, data, row?, column=1, includeHeaders=false)` — `data` = query or array |
| `SpreadsheetAddColumn` | `(workbook, data, startRow=1, startColumn?, insert=false, delimiter=",")` |
| `SpreadsheetGetColumnCount` | `(workbook)` |
| `SpreadsheetSetCellRangeValue` | `(workbook, value, startRow, startColumn, endRow, endColumn)` |
| `SpreadsheetSetCellFormula` | `(workbook, formula, row, column)` — formula in Excel A1 notation |
| `SpreadsheetGetCellFormula` | `(workbook, row, column)` |
| `SpreadsheetGetCellType` | `(workbook, row, column)` |
| `SpreadsheetClearCell` | `(workbook, row, column)` |
| `SpreadsheetClearCellRange` | `(workbook, startRow, startColumn, endRow, endColumn)` |
| `SpreadsheetDeleteRow` / `SpreadsheetDeleteColumn` | `(workbook, row)` / `(workbook, column)` |
| `SpreadsheetDeleteRows` / `SpreadsheetDeleteColumns` | `(workbook, range)` — range like `"2-5,7"` |
| `SpreadsheetShiftRows` / `SpreadsheetShiftColumns` | `(workbook, start, end?, offset=1)` |

### Comments, hyperlinks, media & charts
| Function | Signature |
|---|---|
| `SpreadsheetSetCellComment` | `(workbook, commentOrStruct, row, column)` — struct `{comment, author}` |
| `SpreadsheetSetCellHyperlink` | `(workbook, link, row, column, tooltip?)` |
| `SpreadsheetAddAutofilter` | `(workbook, cellRange)` |
| `SpreadsheetAddImage` | `(workbook, filePath, anchor)` — `anchor` = `"A1"` or `"row,col"` |
| `SpreadsheetAddChart` | `(workbook, { type, series, from, to, title })` — see Charts below |
| `SpreadsheetAddInfo` | `(workbook, struct)` — document properties (title/author/subject/…) |

### Formatting & sizing
| Function | Signature |
|---|---|
| `SpreadsheetFormatCell` | `(workbook, format, row, column)` |
| `SpreadsheetFormatRow` | `(workbook, format, row)` |
| `SpreadsheetFormatColumn` | `(workbook, format, column)` |
| `SpreadsheetFormatCellRange` | `(workbook, format, startRow, startColumn, endRow, endColumn)` |
| `SpreadsheetMergeCells` | `(workbook, startRow, startColumn, endRow, endColumn)` |
| `SpreadsheetAddFreezePane` | `(workbook, freezeColumn, freezeRow)` |
| `SpreadsheetSetColumnWidth` | `(workbook, column, width)` |
| `SpreadsheetSetRowHeight` | `(workbook, row, height)` |

### Sheets
| Function | Signature |
|---|---|
| `SpreadsheetCreateSheet` | `(workbook, sheetName)` |
| `SpreadsheetRenameSheet` | `(workbook, sheetName, sheetNumber)` |
| `SpreadsheetSetActiveSheet` | `(workbook, sheetName)` |
| `SpreadsheetSetActiveSheetNumber` | `(workbook, sheetNumber)` |

### Data interchange
| Function | Signature | Notes |
|---|---|---|
| `SpreadsheetToQuery` | `(workbook)` | Active sheet → query (row 1 = column names) |
| `SpreadsheetToArray` | `(workbook)` | Active sheet → array of row-arrays |
| `SpreadsheetToCsv` | `(workbook, delimiter=",")` | Active sheet → CSV string |
| `SpreadsheetWriteToCsv` | `(workbook, filePath, delimiter=",")` | Active sheet → CSV file |
| `SpreadsheetReadCsv` | `(filePath, delimiter=",")` | CSV file → new workbook |
| `SpreadsheetToJson` | `(workbook, pretty=false)` | Active sheet → JSON array of row objects |
| `SpreadsheetFromJson` | `(json)` | JSON array (objects or arrays) → new workbook |
| `IsSpreadsheetFile` | `(path)` | True for an existing `.xlsx`/`.xls`/`.xlsb`/`.ods` file |

### Introspection getters
`SpreadsheetGetColumnWidth(workbook, column)`, `SpreadsheetGetCellFormat(workbook,
row, column)` (returns a format struct — the same keys `formatCell` accepts, with
colours as `#RRGGBB`), `SpreadsheetGetColumnCount`, plus `GetCellFormula` /
`GetCellType` / `GetCellComment` / `GetCellHyperlink` covered above.

### Row/column visibility
`SpreadsheetSetColumnHidden(workbook, column, hidden)`,
`SpreadsheetSetRowHidden(workbook, row, hidden)`; member-form
`hideColumn/showColumn/hideRow/showRow`, and predicates
`isColumnHidden(column)` / `isRowHidden(row)`.

### Fluent-only convenience methods (member form)
Available on the workbook object in addition to the mapped BIFs above:
`autoSizeColumn(column)`, `autoSize()` (all columns), `freezeRows(n)`,
`freezeCols(n)`, `rowCount()`, `columnCount()`, `toBinary()`,
`sheet(name)` (create-and-select), `selectSheet(nameOrNumber)`,
`headerRow(array)` (write + bold row 1), `fromQuery(q, headers?)`,
`fromArray(a)`, `fromCsv(text, delimiter?)`.

---

## Format struct keys

`formatCell` / `formatRow` / `formatColumn` / `formatCellRange` accept these
(case-insensitive) keys:

| Key(s) | Effect |
|---|---|
| `bold` | Bold font |
| `italic` | Italic font |
| `underline` | Single underline |
| `font`, `fontname` | Font family name |
| `fontsize`, `size` | Font size (points) |
| `color`, `fontcolor` | Font colour (name or hex) |
| `bgcolor`, `fgcolor`, `backgroundcolor` | Solid cell fill (name or hex) |
| `alignment`, `horizontalalignment` | `left` / `center` / `right` / `justify` |
| `verticalalignment` | `top` / `center` (`middle`) / `bottom` |
| `dataformat`, `numberformat` | Excel number-format code (e.g. `"0.00"`, `"yyyy-mm-dd"`) |
| `wraptext` | Wrap long text in the cell |

Example:
```cfml
spreadsheetFormatCell( wb, {
    bold: true, fontsize: 14, color: "white", bgcolor: "##2c3e50",
    alignment: "center", dataformat: "##,##0.00"
}, 1, 1 );
```

---

## Reading & round-tripping

`SpreadsheetRead` opens an existing `.xlsx` into a fully mutable workbook. Because
the engine keeps the whole workbook in memory, you can read a file, change part
of it, and write it back with the untouched content — **including styling,
charts and images** — preserved:

```cfml
wb = spreadsheetRead( expandPath( "/template.xlsx" ) );
spreadsheetSetCellValue( wb, now(), 1, 2 );          // stamp one cell
spreadsheetWrite( wb, expandPath( "/filled.xlsx" ), true );  // everything else intact
```

---

## Worked example: export a query

```cfml
people = queryNew( "id,name,score", "integer,varchar,integer" );
queryAddRow( people, { id: 1, name: "Alice", score: 90 } );
queryAddRow( people, { id: 2, name: "Bob",   score: 75 } );

Spreadsheet( "xlsx" )
    .renameSheet( "People", 1 )
    .addRows( people, 1, 1, true )                        // header + data
    .formatRow( { bold: true, bgcolor: "##dfe6e9" }, 1 )
    .formatColumn( { alignment: "right" }, 3 )            // right-align scores
    .addFreezePane( 0, 1 )
    .autoSize()
    .write( expandPath( "/people.xlsx" ), true );
```

---

## Charts

```cfml
Spreadsheet( "xlsx" )
    .sheet( "Data" )
    .fromQuery( scores, true )
    .addChart( {
        type   : "bar",                       // line | line3d | bar | bar3d | pie | pie3d | doughnut | area
        series : [ "Data!$C$2:$C$10" ],        // one or more sheet-qualified ranges
        from   : "E1",                         // top-left anchor cell
        to     : "L15",                        // bottom-right anchor cell
        title  : "Scores"
    } )
    .write( expandPath( "/chart.xlsx" ), true );
```

## Comments, hyperlinks & images

```cfml
spreadsheetSetCellComment( wb, { comment: "Reviewed", author: "QA" }, 2, 3 );
spreadsheetSetCellHyperlink( wb, "https://example.com", 2, 4, "Open link" );
spreadsheetAddImage( wb, expandPath( "/logo.png" ), "A1" );  // path + anchor cell
```

## The `<cfspreadsheet>` tag

```cfml
<!--- write a query to a file --->
<cfspreadsheet action="write" filename="#expandPath('/out.xlsx')#" query="#myQuery#" overwrite="true">

<!--- read a file back into a query (or a workbook object with name=) --->
<cfspreadsheet action="read" src="#expandPath('/out.xlsx')#" query="result">
<cfspreadsheet action="read" src="#expandPath('/out.xlsx')#" name="wbObject" sheet="1">
```
Supported actions: **`read`**, **`write`**, **`update`** (update currently writes
the file; POI-style in-place merge into an existing file is roadmap). Attributes:
`action`, `src`/`filename`, `name`, `query`, `sheet`, `sheetname`, `format`
(`csv`), `overwrite`.

---

## Formats

| Format | Read | Write |
|---|:-:|:-:|
| `.xlsx` / `.xlsm` (Office Open XML) | ✅ (full round-trip) | ✅ |
| CSV | ✅ (`SpreadsheetReadCsv`) | ✅ (`SpreadsheetWriteToCsv` / `toCsv`) |
| `.xls` (legacy BIFF) | ✅ (via `calamine`, data-only) | ❌ (Excel dropped write support; use `.xlsx`) |
| `.xlsb` | ✅ (via `calamine`, data-only) | ❌ |
| `.ods` (OpenDocument) | ✅ (via `calamine`, data-only) | ❌ (write is roadmap) |

`SpreadsheetRead` routes `.xlsx`/`.xlsm` through the full round-trip engine
(styles preserved) and legacy/foreign formats through `calamine` (values only —
styling is not carried over).

---

## Data validation & conditional formatting

```cfml
// Dropdown list on A2:A10
spreadsheetAddDataValidation( wb, {
    range: "A2:A10", type: "list", formula1: '"Red,Green,Blue"'
} );

// Highlight cells > 100 in yellow bold
spreadsheetAddConditionalFormatting( wb, {
    range: "B2:B100", operator: "greaterThan", value: "100",
    format: { bold: true, bgcolor: "yellow" }
} );
```

`addDataValidation` types: `list` / `whole` / `decimal` / `textLength` / `date` /
`custom`, with operators `between`/`notBetween`/`greaterThan`/`lessThan`/…​.
`addConditionalFormatting` applies a `cellIs` rule (operator + value) with a
format struct.

## Page layout, panes, protection

`SpreadsheetAddSplitPane(workbook, x, y, leftmostColumn, topRow, activePane)`,
`SpreadsheetSetHeader/SetFooter(workbook, left, center, right)` (Excel `&L&C&R`
codes accepted), `SpreadsheetSetPrintOrientation(workbook, "landscape"|"portrait")`,
`SpreadsheetSetFitToPage(workbook, state, pagesWide, pagesHigh)`,
`SpreadsheetSetActiveCell(workbook, row, column)`, `SpreadsheetAddPageBreaks(workbook,
rowBreaks, columnBreaks)`, `SpreadsheetSetRepeatingRows/SetRepeatingColumns(workbook,
range)` (print titles), `SpreadsheetGetCellComment` / `SpreadsheetGetCellHyperlink`.
Pass a `password` to `SpreadsheetWrite(workbook, path, overwrite, password)` for an
encrypted file.

A **colour-scale** conditional format is also available:
`spreadsheetAddConditionalFormatting( wb, { range: "B2:B100", type: "colorScale",
colors: [ "red", "yellow", "green" ] } )` (2- or 3-colour). `addImage` accepts a
**binary** value as well as a file path.

---

## Not implemented — backing-crate limits

These are bounded by the Rust ecosystem, not by wiring:

- **`.ods` writing** (reading works via `calamine`; writing would need `spreadsheet-ods`).
- **Chart types** beyond line/bar/pie/doughnut/area (umya doesn't emit scatter/stock/combo/radar).
- **Reading *styles* from legacy `.xls`/`.xlsb`** — `calamine` reads values only.
- **Row/column grouping / outlines** — umya has no outline-level API.
- **Formula *evaluation*** — no Rust crate computes formulas; a formula cell has no
  value until Excel/LibreOffice opens the file and recalculates.
- **Pixel-accurate auto-size** — a headless engine has no font-metrics renderer, so
  `autoSize` writes Excel's best-fit flag (the app computes widths on open).

---

## Notes & differences

- **Auto-size** is written as an Excel *best-fit* flag; the spreadsheet
  application computes the exact pixel width when the file is opened (a headless
  engine has no font-metrics renderer, unlike POI's use of Java AWT). Column
  widths in the produced file are otherwise as set by `setColumnWidth`.
- **Legacy `.xls` writing is unsupported** — `SpreadsheetNew(name, false)`
  creates an xls-typed workbook, but writing it errors. Modern Excel is `.xlsx`;
  write that.
- Spreadsheet functions are **absent from the wasm builds** by design.

See also **[Compatibility & Status](status.md)** and **[Known Issues](known-issues.md)**.
