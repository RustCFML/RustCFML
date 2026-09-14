<cfscript>
// GH #402 attribution: the four levels of the $initializeMixins body, at three
// CFC widths, so the per-METHOD scaling of the whole-variables-scope copy can
// be told apart from the flat charge.
ITER = int( url.iter ?: 2000 );
WARM = int( url.warm ?: 2000 );

function bench( comp, level, iters ) {
	var i = 0;
	for ( i = 1; i <= WARM; i++ ) { createObject( "component", comp ).init( level ); }
	var t = getTickCount( "nano" );
	for ( i = 1; i <= iters; i++ ) { createObject( "component", comp ).init( level ); }
	return ( getTickCount( "nano" ) - t ) / iters / 1000;
}

writeOutput( "methods |  L0 ctor |  L1 +meta | L2 +core copy | L3 +appends | mixin charge" & chr(10) );
for ( row in [ [ "20", "scripts.perf.cfc_construction.MixinTarget20" ],
               [ "86", "scripts.perf.cfc_construction.MixinTarget" ],
               [ "300", "scripts.perf.cfc_construction.MixinTarget300" ] ] ) {
	l0 = bench( row[2], 0, ITER );
	l1 = bench( row[2], 1, ITER );
	l2 = bench( row[2], 2, ITER );
	l3 = bench( row[2], 3, ITER );
	writeOutput( "  " & row[1] & "    | "
		& numberFormat( l0, "99.999" ) & " | " & numberFormat( l1, "99.999" )
		& " | " & numberFormat( l2, "99.999" ) & " | " & numberFormat( l3, "99.999" )
		& " | " & numberFormat( l3 - l0, "99.999" ) & " us" & chr(10) );
}
</cfscript>
