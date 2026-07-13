<cfscript>
suiteBegin("cfspreadsheet tag");

// The <cfspreadsheet> tag maps to the Spreadsheet* BIFs (read/write/update).
// Guarded so engines without spreadsheet support skip cleanly.
canSpreadsheet = true;
try { _p = spreadsheetNew( "probe", true ); } catch ( any e ) { canSpreadsheet = false; }

tmpDir = getTempDirectory();
ssFile = tmpDir & "/rcfml_sstag_" & getTickCount() & ".xlsx";

people = queryNew( "id,name", "integer,varchar" );
queryAddRow( people, { id = 1, name = "Alice" } );
queryAddRow( people, { id = 2, name = "Bob" } );
</cfscript>

<cfif canSpreadsheet>
    <!--- write a query out to a file via the tag --->
    <cfspreadsheet action="write" filename="#ssFile#" query="#people#" overwrite="true">

    <!--- read it back into a query via the tag --->
    <cfspreadsheet action="read" src="#ssFile#" query="backQuery">

    <!--- read it back into a workbook object via the tag --->
    <cfspreadsheet action="read" src="#ssFile#" name="backObject">

    <cfscript>
        assertTrue( "tag write created file", fileExists( ssFile ) );
        assert( "tag read→query columnList", backQuery.columnList, "id,name" );
        assert( "tag read→query recordCount", backQuery.recordCount, 2 );
        assert( "tag read→query data", backQuery.name[ 2 ], "Bob" );
        assertTrue( "tag read→object", isSpreadsheetObject( backObject ) );
        if ( fileExists( ssFile ) ) { fileDelete( ssFile ); }
    </cfscript>
<cfelse>
    <cfscript>
        assertTrue( "cfspreadsheet tag — spreadsheet support unavailable, skipped", true );
    </cfscript>
</cfif>

<cfscript>
suiteEnd();
</cfscript>
