<cfscript>
if ( !structKeyExists( application, "registry" ) || !structKeyExists( application.registry, "lazySingleton" ) ) {
    writeOutput( "NOT-VISIBLE" );
    abort;
}
writeOutput( "visible;" );
try { writeOutput( "inherited=[" & application.registry.lazySingleton.privateInvoker( "secretAction" ) & "]" ); }
catch ( any e ) { writeOutput( "INHERITED-ERROR=[" & e.message & "]" ); }
try { writeOutput( " own=[" & application.registry.lazySingleton.secretAction() & "]" ); }
catch ( any e ) { writeOutput( " OWN-ERROR=[" & e.message & "]" ); }
</cfscript>
