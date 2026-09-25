<cfscript>
suiteBegin( "Lucee runtime counters: active requests, SessionTracker (Lucee 7.1 verified)" );
// Lucee 7.1: getActiveRequests() counts in-flight requests INCLUDING this one
// (1 for a lone request), and SessionTracker.getSessionCount() is a number.
// Callers such as preside-ext-k8s-essentials compute `getActiveRequests() - 1`,
// which threw when this returned null.
active = getPageContext().getCFMLFactory().getActiveRequests();
assertTrue( "getActiveRequests() is a number", isNumeric( active ) );
assertTrue( "getActiveRequests() counts this request", active >= 1 );
assert( "getActiveRequests() - 1 is arithmetic, not an error", int( active - 1 ) >= 0, true );
count = createObject( "java", "coldfusion.runtime.SessionTracker" ).getSessionCount();
assertTrue( "SessionTracker.getSessionCount() is a number", isNumeric( count ) );
assertTrue( "SessionTracker.getSessionCount() is not negative", count >= 0 );
suiteEnd();
</cfscript>
