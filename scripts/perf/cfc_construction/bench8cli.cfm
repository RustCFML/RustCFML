<cfscript>
// GH #425: CLI-runnable empty-CFC construction loop (for DHAT allocation census).
ITER = 20000;
P = "scripts.perf.cfc_construction.ladder.L0Empty";
t = getTickCount( "nano" );
for ( i = 1; i <= ITER; i++ ) { createObject( "component", P ); }
writeOutput( ITER & " empty-CFC constructions: "
  & numberFormat( ( getTickCount( "nano" ) - t ) / ITER / 1000, "99.999" ) & " us each" & chr(10) );
</cfscript>
