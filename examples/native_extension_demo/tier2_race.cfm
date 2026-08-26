<cfscript>
// Hammered concurrently: the memoiser must compute exactly ONCE across every
// request, and every caller must see the same value.
k = url.k ?: "shared";
writeOutput( demoMemoise( k, randRange( 1000, 9999 ) ) & "|" & demoMemoiseComputations() );
</cfscript>
