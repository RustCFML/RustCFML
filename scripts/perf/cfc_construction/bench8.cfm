<cfscript>
// GH #425: pure empty-CFC construction loop, for the native --profile flamegraph.
// Nothing in the loop but createObject() on a component with an empty body.
ITER = int( url.iter ?: 2000000 );
P = "scripts.perf.cfc_construction.ladder.L0Empty";
t = getTickCount( "nano" );
for ( i = 1; i <= ITER; i++ ) { createObject( "component", P ); }
e = ( getTickCount( "nano" ) - t ) / ITER / 1000;
writeOutput( ITER & " empty-CFC constructions: " & numberFormat( e, "99.999" ) & " us each" & chr(10) );
</cfscript>
