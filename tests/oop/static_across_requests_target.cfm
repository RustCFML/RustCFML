<cfscript>
// Target page for test_static_across_requests.cfm — one request, one bump.
writeOutput( new StaticAcrossRequests().bump() );
</cfscript>
