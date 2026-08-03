<cfscript>
// Target for the <cfdump abort="true"> test: fetched over HTTP so the abort
// ends THAT request instead of the test runner's.
writeOutput( "before-dump" );
data = { a: 1 };
</cfscript>
<cfdump var="#data#" abort="true">
<cfscript>
writeOutput( "after-dump-must-not-appear" );
</cfscript>
