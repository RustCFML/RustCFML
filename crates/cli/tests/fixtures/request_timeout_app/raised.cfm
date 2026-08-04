<cfsetting requestTimeout="9">
<cfscript>
// cfsetting RAISES the limit above the configured 2s, so this 3s sleep must
// finish — the limit is re-read on every check, not latched at request start.
sleep( 3000 );
writeOutput( "RAISED-OK" );
</cfscript>
