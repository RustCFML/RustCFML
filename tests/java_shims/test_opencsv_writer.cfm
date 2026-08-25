<cfscript>
suiteBegin("java shim: com.opencsv.CSVWriter + java.io.FileWriter");

// Preside's CsvWriter.cfc — behind every admin data export and form-builder
// download — builds a FileWriter, wraps it in an opencsv CSVWriter, and streams
// rows. Encoding is the csvFormatRow() builtin.

tmp = getTempDirectory() & "/rustcfml_csvshim_" & createUUID() & ".csv";

fileWriter = CreateObject( "java", "java.io.FileWriter" ).init( tmp );
csv        = CreateObject( "java", "com.opencsv.CSVWriter", [ "/no/such/opencsv.jar" ] ).init( fileWriter, javaCast( "char", "," ) );

csv.writeNext( [ "id", "name", "notes" ] );
csv.writeNext( [ 1, "Simple", "nothing special" ] );
csv.writeNext( [ 2, "Comma, inside", 'Quote " inside' ] );
csv.writeNext( [ 3, "Line" & chr(10) & "break", "tail" ] );
csv.flush();
csv.close();

written = fileRead( tmp );
fileDelete( tmp );

// opencsv's writeNext( String[] ) quotes EVERY field and doubles an embedded
// quote; the record terminator is \n.
expected = '"id","name","notes"' & chr(10)
         & '"1","Simple","nothing special"' & chr(10)
         & '"2","Comma, inside","Quote "" inside"' & chr(10)
         & '"3","Line' & chr(10) & 'break","tail"' & chr(10);

assert( "the file matches opencsv's output byte for byte", written, expected );

// writeAll takes a list of rows.
tmp2 = getTempDirectory() & "/rustcfml_csvshim2_" & createUUID() & ".csv";
csv2 = CreateObject( "java", "com.opencsv.CSVWriter" ).init(
	CreateObject( "java", "java.io.FileWriter" ).init( tmp2 ), javaCast( "char", ";" )
);
csv2.writeAll( [ [ "a", "b" ], [ "c", "d" ] ] );
csv2.close();
assert( "writeAll writes every row with the configured delimiter"
      , fileRead( tmp2 ), '"a";"b"' & chr(10) & '"c";"d"' & chr(10) );
fileDelete( tmp2 );

// Constructing the writer creates/truncates the target, as the JVM does.
tmp3 = getTempDirectory() & "/rustcfml_csvshim3_" & createUUID() & ".csv";
fileWrite( tmp3, "stale content" );
CreateObject( "java", "java.io.FileWriter" ).init( tmp3 );
assert( "constructing a FileWriter truncates an existing file", fileRead( tmp3 ), "" );
// ...unless append mode is asked for.
CreateObject( "java", "java.io.FileWriter" ).init( tmp3, true ).write( "kept" ).close();
w = CreateObject( "java", "java.io.FileWriter" ).init( tmp3, true );
w.write( "+more" );
w.close();
assert( "append mode preserves what was there", fileRead( tmp3 ), "kept+more" );
fileDelete( tmp3 );

// ---- the csvFormatRow() builtin directly ---------------------------------
assert( "csvFormatRow quotes every field by default"
      , csvFormatRow( [ "a", 'b"c', "d,e" ] ), '"a","b""c","d,e"' );
assert( "quoteAll=false quotes only what needs it"
      , csvFormatRow( [ "a", 'b"c', "d,e" ], ",", '"', '"', false ), 'a,"b""c","d,e"' );
assert( "a custom delimiter changes what needs quoting"
      , csvFormatRow( [ "a,b", "c;d" ], ";", '"', '"', false ), 'a,b;"c;d"' );
assert( "a backslash escape dialect is supported"
      , csvFormatRow( [ 'say "hi"' ], ",", '"', "\", false ), '"say \"hi\""' );
assert( "an embedded newline forces quotes even in minimal mode"
      , csvFormatRow( [ "one" & chr(10) & "two" ], ",", '"', '"', false ), '"one' & chr(10) & 'two"' );
assert( "an empty array is an empty record", csvFormatRow( [] ), "" );
assertThrows( "csvFormatRow needs a value", function(){ csvFormatRow(); } );

suiteEnd();
</cfscript>
