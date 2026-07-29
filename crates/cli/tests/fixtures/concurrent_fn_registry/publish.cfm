<cfscript>
// Publish into the PRE-EXISTING nested container, then stay in flight so
// request-end rehoming has not run while the reader below is served.
application.registry.lazySingleton = new Child();
writeOutput( "published;" );
sleep( 4000 );
writeOutput( "done" );
</cfscript>
