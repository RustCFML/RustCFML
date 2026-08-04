<cfscript>
// Well past the 2s budget. The catch MUST NOT fire: Lucee's request timeout
// escapes catch(any), so a framework catch-all can't swallow it.
try {
    sleep( 10000 );
    writeOutput( "SLOW-COMPLETED" );
} catch ( any e ) {
    writeOutput( "SLOW-CAUGHT-" & e.type );
}
</cfscript>
