<cfscript>
// GH #425: ONE construction arm, with warm-up as a parameter, so each engine's
// warm-up convergence can be checked instead of assumed. `cfc` selects width.
ITER = int( url.iter ?: 20000 );
WARM = int( url.warm ?: 20000 );
NAME = url.cfc ?: "MixinTarget20";
P    = "scripts.perf.cfc_construction." & ( NAME EQ "L0Empty" ? "ladder.L0Empty" : NAME );
DOINIT = ( url.init ?: "1" ) EQ "1";
i = 0;
for ( i = 1; i <= WARM; i++ ) { if ( DOINIT ) { createObject( "component", P ).init( 0 ); } else { createObject( "component", P ); } }
t = getTickCount( "nano" );
for ( i = 1; i <= ITER; i++ ) { if ( DOINIT ) { createObject( "component", P ).init( 0 ); } else { createObject( "component", P ); } }
writeOutput( NAME & ": " & numberFormat( ( getTickCount( "nano" ) - t ) / ITER / 1000, "99.999" )
  & " us (warm=" & WARM & ")" & chr(10) );
</cfscript>
