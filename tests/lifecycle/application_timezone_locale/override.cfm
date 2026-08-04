<cfscript>
// setTimeZone()/setLocale() run after the pseudo-constructor, so they must still
// win over this.timezone/this.locale for the rest of the request.
before = getTimeZone() & "/" & getLocale();
setTimeZone( "America/New_York" );
setLocale( "en_US" );
writeOutput( "before=" & before & "|after=" & getTimeZone() & "/" & getLocale() );
</cfscript>
