<cfscript>
// GH #425: construction cost of an EMPTY component, cross-engine.
// Page-scope vars deliberately undeclared (Lucee rejects `var` at page scope).
ITER = int( url.iter ?: 20000 );
WARM = int( url.warm ?: 20000 );
P    = "scripts.perf.cfc_construction.ladder.L0Empty";
i = 0;
for ( i = 1; i <= WARM; i++ ) { createObject( "component", P ); }
t = getTickCount( "nano" );
for ( i = 1; i <= ITER; i++ ) { createObject( "component", P ); }
writeOutput( "empty CFC: " & numberFormat( ( getTickCount( "nano" ) - t ) / ITER / 1000, "99.999" )
  & " us  (warm=" & WARM & " iter=" & ITER & ")" & chr(10) );
</cfscript>
