<cfscript>
// A diagnostic, in PLAIN CFML — no extension involved.
//
// If a memoiser looks broken, run this first. When the counter stays at 1 across
// requests, the `application` scope is not persisting at all, and the cause is
// the application (usually a missing Application.cfc), not the extension. That
// exact confusion cost real time here, which is why this file exists.
lock scope="application" type="exclusive" timeout="5" {
    if ( !structKeyExists( application, "cfmlCounter" ) ) { application.cfmlCounter = 0; }
    application.cfmlCounter++;
}
writeOutput( "plain CFML: application.cfmlCounter = " & application.cfmlCounter
           & " (should INCREASE on each request)" );
</cfscript>
