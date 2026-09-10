<cfscript>
// Target page for test_cache_across_requests.cfm.
switch ( url.op ) {
	case "put":    cachePut( url.key, "value-from-put-request" ); writeOutput( "put" ); break;
	case "get":    v = cacheGet( url.key ); writeOutput( isNull( v ) ? "MISSING" : v ); break;
	case "exists": writeOutput( cacheKeyExists( url.key ) ? "yes" : "no" ); break;
	case "delete": cacheDelete( url.key ); writeOutput( "deleted" ); break;
}
</cfscript>
