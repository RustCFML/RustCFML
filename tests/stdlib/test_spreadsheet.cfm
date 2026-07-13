<cfscript>
suiteBegin("Spreadsheet functions");

// ============================================================
// Native spreadsheet support — Spreadsheet* BIFs + the fluent Spreadsheet()
// builder. Backed by the pure-Rust umya-spreadsheet (read+edit+write .xlsx,
// POI-model round-trip) behind the `spreadsheet` cargo feature.
//
// API is pinned to the ACF/Lucee/BoxLang convention: 1-based row/column,
// workbook as the first arg to the function form, plain-struct styling.
//
// NOTE: Lucee ships spreadsheet as an OPTIONAL extension; ACF has it built in.
// On RustCFML the functions are built in (spreadsheet feature, native builds).
// If the running engine has no spreadsheet support the suite skips cleanly
// rather than reporting false failures.
// ============================================================

canSpreadsheet = true;
try {
    _probe = spreadsheetNew( "probe", true );
} catch ( any e ) {
    canSpreadsheet = false;
}

if ( !canSpreadsheet ) {
    assertTrue( "spreadsheet support not available on this engine — suite skipped", true );
} else {
    tmpDir = getTempDirectory();
    outFile = tmpDir & "/rcfml_ss_" & getTickCount() & ".xlsx";

    // ---- construction + type predicate ----------------------------------
    wb = spreadsheetNew( "Data", true );
    assertTrue( "isSpreadsheetObject(new workbook)", isSpreadsheetObject( wb ) );
    assertFalse( "isSpreadsheetObject(string)", isSpreadsheetObject( "not a workbook" ) );
    assertFalse( "isSpreadsheetObject(struct)", isSpreadsheetObject( {} ) );

    // ---- setCellValue / getCellValue (string + numeric) -----------------
    spreadsheetSetCellValue( wb, "Hello", 1, 1 );
    spreadsheetSetCellValue( wb, "World", 1, 2 );
    spreadsheetSetCellValue( wb, 42, 2, 1 );
    assert( "getCellValue A1 (string)", spreadsheetGetCellValue( wb, 1, 1 ), "Hello" );
    assert( "getCellValue B1 (string)", spreadsheetGetCellValue( wb, 1, 2 ), "World" );
    assert( "getCellValue A2 (numeric)", spreadsheetGetCellValue( wb, 2, 1 ), "42" );

    // ---- info struct ----------------------------------------------------
    info = spreadsheetInfo( wb );
    assert( "info.sheets == 1", info.sheets, 1 );
    assert( "info.columncount == 2", info.columncount, 2 );
    assert( "info.rowcount == 2", info.rowcount, 2 );
    assert( "info.sheetnames first == Data", info.sheetnames[ 1 ], "Data" );

    // ---- createSheet + renameSheet --------------------------------------
    spreadsheetCreateSheet( wb, "Second" );
    assert( "after createSheet sheets == 2", spreadsheetInfo( wb ).sheets, 2 );
    spreadsheetRenameSheet( wb, "Renamed", 2 );
    assert( "renameSheet applied", spreadsheetInfo( wb ).sheetnames[ 2 ], "Renamed" );

    // ---- addRows from a query (with header row) -------------------------
    q = queryNew( "id,name,score", "integer,varchar,integer" );
    queryAddRow( q, { id = 1, name = "Alice", score = 90 } );
    queryAddRow( q, { id = 2, name = "Bob",   score = 75 } );
    wbq = spreadsheetNew( "Q", true );
    spreadsheetAddRows( wbq, q, 1, 1, true );
    assert( "addRows header A1", spreadsheetGetCellValue( wbq, 1, 1 ), "id" );
    assert( "addRows header B1", spreadsheetGetCellValue( wbq, 1, 2 ), "name" );
    assert( "addRows data B2",   spreadsheetGetCellValue( wbq, 2, 2 ), "Alice" );
    assert( "addRows data B3",   spreadsheetGetCellValue( wbq, 3, 2 ), "Bob" );

    // ---- addColumn ------------------------------------------------------
    spreadsheetAddColumn( wbq, [ "x", "y", "z" ], 1, 4, false );
    assert( "addColumn D1", spreadsheetGetCellValue( wbq, 1, 4 ), "x" );
    assert( "addColumn D3", spreadsheetGetCellValue( wbq, 3, 4 ), "z" );

    // ---- formatting: does not throw, returns the workbook (fluent) ------
    assertTrue( "formatRow returns workbook (fmt-first order)",
        isSpreadsheetObject( spreadsheetFormatRow( wbq, { bold = true, bgcolor = "lightblue" }, 1 ) ) );
    assertTrue( "formatCell returns workbook",
        isSpreadsheetObject( spreadsheetFormatCell( wbq, { color = "red" }, 2, 2 ) ) );
    assertTrue( "mergeCells returns workbook",
        isSpreadsheetObject( spreadsheetMergeCells( wbq, 5, 1, 5, 3 ) ) );
    assertTrue( "addFreezePane returns workbook",
        isSpreadsheetObject( spreadsheetAddFreezePane( wbq, 0, 1 ) ) );
    assertTrue( "setColumnWidth returns workbook",
        isSpreadsheetObject( spreadsheetSetColumnWidth( wbq, 1, 25 ) ) );
    assertTrue( "setRowHeight returns workbook",
        isSpreadsheetObject( spreadsheetSetRowHeight( wbq, 1, 30 ) ) );

    // ---- write to disk --------------------------------------------------
    spreadsheetWrite( wbq, outFile, true );
    assertTrue( "write created file", fileExists( outFile ) );
    assertTrue( "written file is non-empty", getFileInfo( outFile ).size > 0 );

    // ---- read → round-trip: data survives -------------------------------
    reopened = spreadsheetRead( outFile );
    assertTrue( "spreadsheetRead returns a workbook", isSpreadsheetObject( reopened ) );
    assert( "round-trip header A1", spreadsheetGetCellValue( reopened, 1, 1 ), "id" );
    assert( "round-trip data B2", spreadsheetGetCellValue( reopened, 2, 2 ), "Alice" );

    // ---- edit → rewrite → read: edit lands, other data intact -----------
    spreadsheetSetCellValue( reopened, "EDITED", 1, 1 );
    editFile = tmpDir & "/rcfml_ss_edit_" & getTickCount() & ".xlsx";
    spreadsheetWrite( reopened, editFile, true );
    reopened2 = spreadsheetRead( editFile );
    assert( "edited cell persisted", spreadsheetGetCellValue( reopened2, 1, 1 ), "EDITED" );
    assert( "untouched cell intact", spreadsheetGetCellValue( reopened2, 2, 2 ), "Alice" );

    // ---- readBinary → valid binary --------------------------------------
    bin = spreadsheetReadBinary( wbq );
    assertTrue( "readBinary returns binary", isBinary( bin ) );

    // ---- fluent builder chain -------------------------------------------
    fluentFile = tmpDir & "/rcfml_ss_fluent_" & getTickCount() & ".xlsx";
    fwb = spreadsheet( "xlsx" )
        .renameSheet( "Results", 1 )
        .setCellValue( "chain-a", 1, 1 )          // (value, row, col)
        .setCellValue( "chain-b", 1, 2 )
        .formatCell( 1, 1, { bold = true } )       // BoxLang coords-first order
        .addFreezePane( 0, 1 )
        .autoSize();
    assertTrue( "fluent chain yields a workbook", isSpreadsheetObject( fwb ) );
    assert( "fluent A1", fwb.getCellValue( 1, 1 ), "chain-a" );
    assert( "fluent sheet renamed", fwb.info().sheetnames[ 1 ], "Results" );
    fwb.write( fluentFile, true );
    assertTrue( "fluent write created file", fileExists( fluentFile ) );

    // ---- formulas & cell type ------------------------------------------
    wf = spreadsheetNew( "F", true );
    spreadsheetSetCellValue( wf, 10, 1, 1 );
    spreadsheetSetCellValue( wf, 20, 2, 1 );
    spreadsheetSetCellFormula( wf, "SUM(A1:A2)", 3, 1 );
    assert( "getCellFormula", spreadsheetGetCellFormula( wf, 3, 1 ), "SUM(A1:A2)" );
    assert( "getCellType numeric", spreadsheetGetCellType( wf, 1, 1 ), "n" );

    // ---- delete / clear -------------------------------------------------
    wd = spreadsheetNew( "D", true );
    spreadsheetAddRows( wd, q, 1, 1, true );          // header + Alice + Bob
    spreadsheetDeleteRow( wd, 2 );                     // remove Alice's row
    assert( "deleteRow shifts Bob up", spreadsheetGetCellValue( wd, 2, 2 ), "Bob" );
    spreadsheetClearCell( wd, 2, 2 );
    assert( "clearCell empties cell", spreadsheetGetCellValue( wd, 2, 2 ), "" );

    // ---- toQuery / toArray / toCsv round-trip ---------------------------
    wt = spreadsheetNew( "T", true );
    spreadsheetAddRows( wt, q, 1, 1, true );
    qOut = spreadsheetToQuery( wt );
    assert( "toQuery columnList", qOut.columnList, "id,name,score" );
    assert( "toQuery recordCount", qOut.recordCount, 2 );
    assert( "toQuery data", qOut.name[ 1 ], "Alice" );
    arrOut = spreadsheetToArray( wt );
    assert( "toArray row count", arrayLen( arrOut ), 3 );        // header + 2 rows
    assert( "toArray cell", arrOut[ 1 ][ 1 ], "id" );
    csvOut = spreadsheetToCsv( wt );
    assertTrue( "toCsv has header", csvOut contains "id,name,score" );
    assertTrue( "toCsv has data", csvOut contains "Alice" );

    // ---- comments / hyperlinks / autofilter / chart (fluent, no-throw) --
    wr = spreadsheet( "xlsx" ).fromQuery( q, true );
    assertTrue( "setCellComment fluent", isSpreadsheetObject(
        wr.setCellComment( { comment = "note", author = "t" }, 2, 2 ) ) );
    assertTrue( "setCellHyperlink fluent", isSpreadsheetObject(
        wr.setCellHyperlink( "https://example.com", 2, 3 ) ) );
    assertTrue( "addAutofilter fluent", isSpreadsheetObject(
        wr.addAutofilter( "A1:C1" ) ) );
    assertTrue( "addChart fluent", isSpreadsheetObject(
        wr.addChart( { type = "bar", series = [ "T!$C$2:$C$3" ], from = "E1", to = "L15", title = "S" } ) ) );

    // ---- CSV file round-trip + isSpreadsheetFile ------------------------
    csvFile = tmpDir & "/rcfml_ss_" & getTickCount() & ".csv";
    spreadsheetWriteToCsv( wt, csvFile );
    assertTrue( "writeToCsv created file", fileExists( csvFile ) );
    csvWb = spreadsheetReadCsv( csvFile );
    assert( "readCsv A1", spreadsheetGetCellValue( csvWb, 1, 1 ), "id" );
    assert( "readCsv data B2", spreadsheetGetCellValue( csvWb, 2, 2 ), "Alice" );
    xlsxFile = tmpDir & "/rcfml_ssf_" & getTickCount() & ".xlsx";
    spreadsheetWrite( wt, xlsxFile, true );
    assertTrue( "isSpreadsheetFile(.xlsx)", isSpreadsheetFile( xlsxFile ) );
    assertFalse( "isSpreadsheetFile(.txt)", isSpreadsheetFile( "/nope/not-a-sheet.txt" ) );

    // ---- comment / hyperlink getters -----------------------------------
    wc = spreadsheetNew( "C", true );
    spreadsheetSetCellValue( wc, "x", 1, 1 );
    spreadsheetSetCellComment( wc, { comment = "hi", author = "me" }, 1, 1 );
    spreadsheetSetCellHyperlink( wc, "https://example.com", 2, 1, "tip" );
    cmt = spreadsheetGetCellComment( wc, 1, 1 );
    assert( "getCellComment text", cmt.comment, "hi" );
    assert( "getCellComment author", cmt.author, "me" );
    assert( "getCellHyperlink", spreadsheetGetCellHyperlink( wc, 2, 1 ), "https://example.com" );

    // ---- layout / validation / conditional formatting (fluent, no-throw)-
    wl = spreadsheet( "xlsx" ).sheet( "L" ).headerRow( [ "A", "B" ] );
    assertTrue( "addSplitPane", isSpreadsheetObject( wl.addSplitPane( 2000, 2000, 3, 3, "LOWER_RIGHT" ) ) );
    assertTrue( "setHeader/setFooter", isSpreadsheetObject( wl.setHeader( "L", "C", "R" ).setFooter( "", "&P", "" ) ) );
    assertTrue( "setPrintOrientation/setFitToPage", isSpreadsheetObject( wl.setPrintOrientation( "landscape" ).setFitToPage( true, 1, 1 ) ) );
    assertTrue( "setColumnHidden", isSpreadsheetObject( wl.setColumnHidden( 2, true ) ) );
    assertTrue( "isColumnHidden true", wl.isColumnHidden( 2 ) );
    assertTrue( "addDataValidation", isSpreadsheetObject(
        spreadsheetAddDataValidation( wl, { range = "A2:A10", type = "list", formula1 = """Red,Green""" } ) ) );
    assertTrue( "addConditionalFormatting", isSpreadsheetObject(
        spreadsheetAddConditionalFormatting( wl, { range = "A1:A9", operator = "greaterThan", value = "5", format = { bold = true, bgcolor = "yellow" } } ) ) );

    // ---- password-protected write --------------------------------------
    pwFile = tmpDir & "/rcfml_sspw_" & getTickCount() & ".xlsx";
    spreadsheetWrite( wc, pwFile, true, "secret" );
    assertTrue( "password write created file", fileExists( pwFile ) );

    // ---- legacy .xls read (calamine) — uses a committed fixture ---------
    legacyFixture = getDirectoryFromPath( getCurrentTemplatePath() ) & "fixtures/legacy.xls";
    if ( fileExists( legacyFixture ) ) {
        lwb = spreadsheetRead( legacyFixture );
        assertTrue( "legacy .xls read → workbook", isSpreadsheetObject( lwb ) );
        assert( "legacy .xls A1", spreadsheetGetCellValue( lwb, 1, 1 ), "id" );
        assert( "legacy .xls B2", spreadsheetGetCellValue( lwb, 2, 2 ), "Alice" );
        assert( "legacy .xls sheet name", lwb.info().sheetnames[ 1 ], "Legacy" );
    }

    // ---- getters: column width & cell format ---------------------------
    wg = spreadsheetNew( "G", true );
    spreadsheetSetCellValue( wg, "x", 1, 1 );
    spreadsheetSetColumnWidth( wg, 1, 27 );
    spreadsheetFormatCell( wg, { bold = true, color = "red", bgcolor = "yellow", alignment = "center", dataformat = "0.00" }, 1, 1 );
    assert( "getColumnWidth", spreadsheetGetColumnWidth( wg, 1 ), 27 );
    gf = spreadsheetGetCellFormat( wg, 1, 1 );
    assertTrue( "getCellFormat bold", gf.bold );
    assert( "getCellFormat color", gf.color, "##FF0000" );
    assert( "getCellFormat bgcolor", gf.bgcolor, "##FFFF00" );
    assert( "getCellFormat alignment", gf.alignment, "center" );
    assert( "getCellFormat dataformat", gf.dataformat, "0.00" );

    // ---- layout: active cell / page breaks / repeating (fluent) --------
    assertTrue( "setActiveCell/pageBreaks/repeating fluent", isSpreadsheetObject(
        wg.setActiveCell( 2, 2 ).addPageBreaks( [ 2 ], [ 1 ] ).setRepeatingRows( "$1:$1" ).setRepeatingColumns( "$A:$A" ) ) );

    // ---- colour-scale conditional formatting ---------------------------
    assertTrue( "colorScale CF", isSpreadsheetObject(
        spreadsheetAddConditionalFormatting( wg, { range = "A1:A9", type = "colorScale", colors = [ "red", "yellow", "green" ] } ) ) );

    // ---- JSON interchange round-trip -----------------------------------
    wjson = spreadsheet( "xlsx" ).fromQuery( q, true );
    jsonStr = spreadsheetToJson( wjson );
    assertTrue( "toJson is a JSON array", jsonStr contains "[{" );
    wjback = spreadsheetFromJson( jsonStr );
    assert( "fromJson header A1", spreadsheetGetCellValue( wjback, 1, 1 ), "id" );
    assert( "fromJson data B2", spreadsheetGetCellValue( wjback, 2, 2 ), "Alice" );

    // ---- addImage from binary ------------------------------------------
    if ( isImage( imageNew( "", 4, 4, "rgb", "red" ) ) ) {
        imgBlob = imageGetBlob( imageNew( "", 4, 4, "rgb", "red" ) );
        assertTrue( "addImage(binary)", isSpreadsheetObject( wg.addImage( imgBlob, "C3" ) ) );
    }

    // ---- fluent load ---------------------------------------------------
    loadFile = tmpDir & "/rcfml_ssload_" & getTickCount() & ".xlsx";
    spreadsheet( "xlsx" ).fromQuery( q, true ).write( loadFile, true );
    loaded = spreadsheet( "xlsx" ).load( loadFile );
    assert( "fluent load A1", loaded.getCellValue( 1, 1 ), "id" );
    if ( fileExists( loadFile ) ) { fileDelete( loadFile ); }

    // ---- cleanup --------------------------------------------------------
    if ( fileExists( csvFile ) ) { fileDelete( csvFile ); }
    if ( fileExists( xlsxFile ) ) { fileDelete( xlsxFile ); }
    if ( fileExists( pwFile ) ) { fileDelete( pwFile ); }
    if ( fileExists( outFile ) ) { fileDelete( outFile ); }
    if ( fileExists( editFile ) ) { fileDelete( editFile ); }
    if ( fileExists( fluentFile ) ) { fileDelete( fluentFile ); }
}

suiteEnd();
</cfscript>
