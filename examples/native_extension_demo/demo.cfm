<cfscript>
// A worked tour of a .rcx extension. Build and install it first:
//
//   rustcfml ext build examples/native_extension_demo
//   rustcfml ext install demo-0.1.0.rcx --dir examples/native_extension_demo/extensions
//   rustcfml examples/native_extension_demo/demo.cfm
//
// Note that the engine running this is a STOCK binary. Nothing was rebuilt.

writeOutput( "— functions —" & chr(10) );
writeOutput( demoGreet() & chr(10) );
writeOutput( demoGreet( "there" ) & chr(10) );

writeOutput( chr(10) & "— arrays and structs —" & chr(10) );
writeDump( demoStats( [ 4, 8, 15, 16, 23, 42 ] ) );

writeOutput( chr(10) & "— queries, both directions —" & chr(10) );
q = demoBuildQuery( 5 );
writeOutput( "the extension built a " & q.recordCount & "-row query with columns: " & q.columnList & chr(10) );
writeDump( demoSummariseQuery( q, "square" ) );

sales = queryNew( "region,amount", "varchar,decimal",
                  [ [ "North", 1200.50 ], [ "South", 990.25 ], [ "East", 1710.00 ] ] );
writeDump( demoSummariseQuery( sales, "amount" ) );

writeOutput( chr(10) & "— binary —" & chr(10) );
writeOutput( "checksum: " & demoChecksum( toBinary( toBase64( "hello world" ) ) ) & chr(10) );

writeOutput( chr(10) & "— stateful classes —" & chr(10) );
t = demoTally( 10 );
writeOutput( "bump(by=5) -> " & t.bump( by = 5 ) & chr(10) );
// Mutators return the receiver, so they chain — and `describe()` proves it is
// the SAME object, not a copy.
writeOutput( "chained    -> " & t.label( text = "widgets" ).describe() & chr(10) );
writeOutput( "reset      -> " & t.reset().describe() & chr(10) );

o = createObject( "rust", "Tally", 100 );
writeOutput( "createObject -> " & o.value() & chr(10) );

writeOutput( chr(10) & "— errors —" & chr(10) );
try {
    demoFail();
} catch ( demo.deliberate e ) {
    writeOutput( "caught by custom type: " & e.message & chr(10) );
}
try {
    t.bump( nope = 1 );
} catch ( any e ) {
    writeOutput( "bad argument name    : " & e.message & chr(10) );
}
try {
    demoStats( [ 1, "not a number" ] );
} catch ( any e ) {
    writeOutput( "bad element          : " & e.message & chr(10) );
}
</cfscript>
