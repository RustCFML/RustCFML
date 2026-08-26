<cfscript>
// Tier 2 — an extension that can see the running application.
//
//   rustcfml ext build examples/native_extension_demo
//   rustcfml ext install demo-0.1.0.rcx --dir examples/native_extension_demo/extensions
//   rustcfml --serve . --port 8500      → /examples/native_extension_demo/tier2.cfm
//
// Needs serve mode: the application and server scopes, and the lock registry,
// only exist there.

writeOutput( "— unqualified reads use CFML's own resolution order —" & chr(10) );
request.who = "from request";
writeOutput( "demoRequestVar( 'who' ) = " & demoRequestVar( "who" ) & chr(10) );

writeOutput( chr(10) & "— writing a shared scope unlocked is refused —" & chr(10) );
try {
    demoUnlockedWrite( "sneaky" );
    writeOutput( "FAIL: the unlocked write was allowed" & chr(10) );
} catch ( any e ) {
    writeOutput( "refused: " & e.message & chr(10) );
}
writeOutput( "application.sneaky exists? " & structKeyExists( application, "sneaky" ) & chr(10) );

writeOutput( chr(10) & "— memoising into application, the right way —" & chr(10) );
writeOutput( "first call  : " & demoMemoise( "answer", 42 ) & chr(10) );
writeOutput( "second call : " & demoMemoise( "answer", 99 ) & " (cached, so still 42)" & chr(10) );
writeOutput( "computations: " & demoMemoiseComputations() & chr(10) );
writeOutput( "visible to CFML as application.answer = " & application.answer & chr(10) );

writeOutput( chr(10) & "— and CFML's own lock excludes the extension —" & chr(10) );
lock scope="application" type="exclusive" timeout="5" {
    // Inside a CFML lock on the same scope, a nested extension write is
    // REENTRANT for this request (as <cflock> is), so this succeeds.
    demoMemoise( "insideLock", 7 );
    writeOutput( "reentrant write inside <cflock>: " & application.insideLock & chr(10) );
}
</cfscript>
