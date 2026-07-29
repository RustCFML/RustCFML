<cfscript>
// Establishes the nested container so its inner storage is shared with the
// persisted application snapshot — that sharing is what makes a later
// mid-request write visible to concurrent requests.
application.registry = {};
writeOutput( "registry-established" );
</cfscript>
