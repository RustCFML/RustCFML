<cfscript>
// Long enough that the test can reliably signal while this is mid-flight.
sleep( 3000 );
writeOutput( "slow request completed" );
</cfscript>
