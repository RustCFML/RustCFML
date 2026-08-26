<cfscript>
suiteBegin("Named arguments on native-class methods");

// ============================================================
// Methods on a Rust-backed native object (Spreadsheet, HtmlDocument, Pdf, and
// any class a native module registers) must bind NAMED arguments by name.
//
// Before v0.635.0 the names were dropped and the values passed in call-site
// order, so `renameSheet( sheetNumber=1, sheetName="Zed" )` renamed the sheet
// to "1" — a silent wrong answer, not an error. A native class now declares
// its parameter names (`CfmlNative::method_params`) and the VM reorders; a
// class that declares nothing REFUSES a named call rather than misbinding it.
// ============================================================

wb = Spreadsheet( "xlsx" );

// Names honoured regardless of call-site order.
wb.renameSheet( sheetNumber=1, sheetName="Zed" );
assert( "named args bind by name, not by position", wb.info().sheetnames[ 1 ], "Zed" );

// Positional calls are untouched.
wb.renameSheet( "Positional", 1 );
assert( "positional call unchanged", wb.info().sheetnames[ 1 ], "Positional" );

// A gap in the middle of the parameter list is allowed.
wb.setCellValue( row = 2, column = 3, value = "hi" );
assert( "out-of-order named args on a 4-param method", wb.getCellValue( 2, 3 ), "hi" );

// A struct-valued parameter binds by name like any other.
wb.formatRow( row = 2, format = { bold = true } );
assert( "struct argument binds by name", wb.getCellFormat( 2, 3 ).bold, true );

// argumentCollection spreads into named parameters.
collArgs = { sheetName = "ViaColl", sheetNumber = 1 };
wb.renameSheet( argumentCollection = collArgs );
assert( "argumentCollection spreads by name", wb.info().sheetnames[ 1 ], "ViaColl" );

// An unknown argument name is an error naming the valid ones — never a silent
// bind into the first free slot.
badName = "";
try {
    wb.renameSheet( sheetNam = "typo" );
} catch ( any e ) {
    badName = e.message;
}
assertTrue( "unknown argument name is rejected", findNoCase( "sheetNam", badName ) GT 0 );
assertTrue( "and the error lists the valid names", findNoCase( "sheetNumber", badName ) GT 0 );

// A method that takes no arguments says so.
noArgs = "";
try {
    wb.info( foo = 1 );
} catch ( any e ) {
    noArgs = e.message;
}
assertTrue( "no-argument method rejects a named arg", findNoCase( "no arguments", noArgs ) GT 0 );

// The same rule holds for the other native classes.
doc = HtmlDocument( "<p id='x'>hello <b>world</b></p>" );
el  = doc.select( selector = "##x" )[ 1 ];
assert( "HtmlDocument.select( selector= )", doc.text( element = el ), "hello world" );

suiteEnd();
</cfscript>
