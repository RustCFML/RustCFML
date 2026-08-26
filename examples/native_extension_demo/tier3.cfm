<cfscript>
// Tier 3 — an extension that can run CFML.
//
//   rustcfml ext build examples/native_extension_demo
//   rustcfml ext install demo-0.1.0.rcx --dir examples/native_extension_demo/extensions
//   rustcfml --serve . --port 8500   → /tier3.cfm

writeOutput( "— calling a CFML closure from Rust —" & chr(10) );
triple = function( n ) { return n * 3; };
writeOutput( "demoApply( triple, 14 ) = " & demoApply( triple, 14 ) & chr(10) );

writeOutput( chr(10) & "— calling a CFML builtin from Rust, per element —" & chr(10) );
writeOutput( "demoSort( [pear, apple, fig] ) = " & arrayToList( demoSort( [ "pear", "apple", "fig" ] ) ) & chr(10) );

writeOutput( chr(10) & "— instantiating a CFC, injecting into it, calling it —" & chr(10) );
writeOutput( demoUseComponent( "Greeter", "hello" ) & chr(10) );

writeOutput( chr(10) & "— reading a CFC's metadata (what annotation-driven DI needs) —" & chr(10) );
writeDump( demoComponentAnnotations( "Greeter" ) );

writeOutput( chr(10) & "— writing page output from Rust —" & chr(10) );
demoEmit( "…this line was written by the extension." & chr(10) );

writeOutput( chr(10) & "— re-entrancy: CFML → extension → CFML → extension —" & chr(10) );
// The callback calls BACK into the extension, so the round trip is
// CFML → demoApply → this closure → demoGreet → back out. Under an exclusive
// dispatch guard the nested extension call would deadlock.
nested = function( n ) { return demoGreet( "level " & n ); };
writeOutput( demoApply( nested, 2 ) & chr(10) );
</cfscript>
