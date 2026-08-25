<cfscript>
suiteBegin("java shim: Apache POI over the native spreadsheet engine");

// CFML spreadsheet libraries ship POI as a jar and drive its object graph
// directly. The adapter maps that graph onto RustCFML's Spreadsheet* builtins as
// coordinate handles: a Sheet is a workbook + sheet index, a Row adds a row, a
// Cell adds a column, and every mutation is the matching builtin.
//
// POI is 0-based; the builtins are CFML's 1-based. Everything below is written
// in POI's numbering, exactly as a POI-driving library would.

tmp = getTempDirectory() & "/rustcfml_poi_" & createUUID() & ".xlsx";

wb = CreateObject( "java", "org.apache.poi.xssf.usermodel.XSSFWorkbook" ).init();

// A fresh POI workbook has NO sheets — unlike the engine, which always has one.
assert( "a new workbook starts with no sheets", wb.getNumberOfSheets(), 0 );

sheet = wb.createSheet( "Data" );
assert( "createSheet is the first sheet, not a second one", wb.getNumberOfSheets(), 1 );
assert( "and it carries the requested name", wb.getSheetName( 0 ), "Data" );
assert( "getSheetIndex finds it by name", wb.getSheetIndex( "Data" ), 0 );
assert( "getSheetIndex returns -1 when there is no match", wb.getSheetIndex( "Nope" ), -1 );
assertNull( "getSheet returns null for an unknown name", wb.getSheet( "Nope" ) );

sheet2 = wb.createSheet( "More" );
assert( "a second createSheet really does add one", wb.getNumberOfSheets(), 2 );
wb.setActiveSheet( 0 );

// POI's sheet-name rules are enforced, as WorkbookUtil does.
assertThrows( "a duplicate sheet name is rejected", function(){ wb.createSheet( "Data" ); } );
util = CreateObject( "java", "org.apache.poi.ss.util.WorkbookUtil" );
assertThrows( "an illegal character in a sheet name is rejected", function(){ util.validateSheetName( "a/b" ); } );
assertThrows( "an over-long sheet name is rejected", function(){ util.validateSheetName( repeatString( "x", 32 ) ); } );
util.validateSheetName( "Perfectly Fine" );
assertTrue( "a legal sheet name validates", true );
assert( "createSafeSheetName scrubs the illegal characters", util.createSafeSheetName( "a/b:c" ), "a b c" );

// ---- values, through the row/cell handles --------------------------------
header = sheet.createRow( 0 );
header.createCell( 0 ).setCellValue( "Name" );
header.createCell( 1 ).setCellValue( "Qty" );
body = sheet.createRow( 1 );
body.createCell( 0 ).setCellValue( "Widget" );
body.createCell( 1 ).setCellValue( 7 );

assert( "a value written through a POI cell reads back", header.createCell( 0 ).getStringCellValue(), "Name" );
assert( "a numeric value reads back too", body.createCell( 1 ).getStringCellValue(), "7" );
assert( "getColumnIndex reports POI's 0-based column", body.createCell( 1 ).getColumnIndex(), 1 );
assert( "getRowNum reports POI's 0-based row", body.getRowNum(), 1 );

// getRow is null for a row that was never created — callers branch on it.
assertNull( "getRow past the end is null", sheet.getRow( 99 ) );

// ---- the style accumulator ------------------------------------------------
// POI is configure-then-assign; nothing reaches the workbook until setCellStyle.
style = wb.createCellStyle();
font  = wb.createFont();
font.setBold( true );
style.setFont( font );
header.createCell( 0 ).setCellStyle( style );

assertTrue( "the assigned style reached the cell"
          , header.createCell( 0 ).getCellStyle().getBold() ?: false );
assertFalse( "and a cell it was never assigned to is untouched"
           , body.createCell( 0 ).getCellStyle().getBold() ?: false );

// getClass().getCanonicalName() is how libraries type-check a style, so it must
// report the concrete POI class, not the adapter's internal key.
assert( "a style reports its concrete POI class"
      , style.getClass().getCanonicalName(), "org.apache.poi.xssf.usermodel.XSSFCellStyle" );
assert( "and so does the workbook"
      , wb.getClass().getCanonicalName(), "org.apache.poi.xssf.usermodel.XSSFWorkbook" );

// An UNSET font property answers null, and a setter ignores null. That is what
// makes the clone-a-font-then-modify-it idiom safe: copying an empty font must
// not stamp POI's defaults (Calibri, 11pt, black) onto every cell it touches.
blank = wb.getFontAt( 0 );
assertNull( "an unset font property is null, not a default", blank.getFontName() );
fresh = wb.createFont();
fresh.setFontName( blank.getFontName() );   // copying nothing...
fresh.setBold( true );
freshStyle = wb.createCellStyle();
freshStyle.setFont( fresh );
thirdCell = sheet.createRow( 2 ).createCell( 0 );
thirdCell.setCellValue( "x" );
thirdCell.setCellStyle( freshStyle );
assertTrue( "the explicitly set property is applied", thirdCell.getCellStyle().getBold() ?: false );
assertTrue( "and copying an unset one did not invent a value"
          , isNull( fresh.getFontName() ) );

// setFont MERGES, so building a style up one property at a time composes.
multi = wb.createCellStyle();
f1 = wb.createFont(); f1.setBold( true );   multi.setFont( f1 );
f2 = wb.createFont(); f2.setItalic( true ); multi.setFont( f2 );
assertTrue( "an incrementally built style keeps the earlier property", multi.getBold() ?: false );

// ---- iterators ------------------------------------------------------------
// cellIterator yields only the cells that physically exist, so styling "the
// cells in this row" does not also style the blank tail.
it = header.cellIterator();
seen = 0;
while ( it.hasNext() ) { it.next(); seen++; }
assert( "cellIterator yields the populated cells only", seen, 2 );
assertFalse( "and is exhausted afterwards", it.hasNext() );
assertThrows( "next() past the end throws", function(){ it.next(); } );

// ---- freeze pane, autosize, write ----------------------------------------
sheet.createFreezePane( 0, 1 );
sheet.autoSizeColumn( 0 );

out = CreateObject( "java", "java.io.FileOutputStream" ).init( tmp );
wb.write( out );
out.flush();

assertTrue( "write produced a file", fileExists( tmp ) );
readBack = spreadsheetRead( tmp );
info = spreadsheetInfo( readBack );
assert( "the written workbook has the sheets it was given", info.sheets, 2 );
assert( "the first sheet kept its name", info.sheetnames[ 1 ], "Data" );
assert( "and its values", spreadsheetGetCellValue( readBack, 2, 1 ), "Widget" );
fileDelete( tmp );

// Writing needs a real target: POI streams to anything, this adapter needs a path.
assertThrows( "write without a file-backed stream is refused", function(){ wb.write( "not a stream" ); } );

// Legacy binary .xls: the engine reads that format but cannot write it. Rather
// than fail the caller's export, an HSSFWorkbook is backed by xlsx and the
// substitution is warned about on stderr — the file carries whatever name the
// caller chose, but the bytes are xlsx. (Preside's form-builder export asks for
// .xls; this is what keeps it working until that is changed upstream.)
hssf = CreateObject( "java", "org.apache.poi.hssf.usermodel.HSSFWorkbook" ).init();
hssfSheet = hssf.createSheet( "Legacy" );
hssfSheet.createRow( 0 ).createCell( 0 ).setCellValue( "still written" );

// getClass() keeps reporting HSSFWorkbook on purpose: libraries branch on it to
// choose which cell-style and colour classes to use, and those branches must
// stay consistent with what the style accumulators report. Only the bytes differ.
assert( "an HSSF workbook still identifies as HSSF"
      , hssf.getClass().getCanonicalName(), "org.apache.poi.hssf.usermodel.HSSFWorkbook" );
assert( "and its styles as HSSF styles too"
      , hssf.createCellStyle().getClass().getCanonicalName()
      , "org.apache.poi.hssf.usermodel.HSSFCellStyle" );

xlsPath = getTempDirectory() & "/rustcfml_poi_" & createUUID() & ".xls";
xlsOut = CreateObject( "java", "java.io.FileOutputStream" ).init( xlsPath );
hssf.write( xlsOut );
xlsOut.flush();
assertTrue( "an HSSF workbook writes rather than failing", fileExists( xlsPath ) );

// The bytes are xlsx. spreadsheetRead() dispatches on the EXTENSION, so reading
// this file back through a `.xls` name is not expected to work — the substitution
// is a write-side accommodation for callers that ask for a format this engine
// cannot produce, and deliberately stops there rather than changing how the core
// engine reads files.
assertTrue( "the substituted file carries xlsx bytes"
          , left( toString( fileReadBinary( xlsPath ) ), 2 ) == "PK" );
fileDelete( xlsPath );

// Out of scope is refused, not silently dropped.
assertThrows( "an unmodelled POI method throws", function(){ sheet.getPrintSetup(); } );

suiteEnd();
</cfscript>
