<cfscript>
// Construction cost vs declared method count, both engines. Isolated: no
// mixins, no metadata, just createObject().init().
ITER = int( url.iter ?: 5000 );
WARM = int( url.warm ?: 5000 );
function bench( comp, iters ) {
	var i = 0;
	for ( i = 1; i <= WARM; i++ ) { createObject( "component", comp ).init( 0 ); }
	var t = getTickCount( "nano" );
	for ( i = 1; i <= iters; i++ ) { createObject( "component", comp ).init( 0 ); }
	return ( getTickCount( "nano" ) - t ) / iters / 1000;
}
for ( row in [ [ "20", "scripts.perf.cfc_construction.MixinTarget20" ],
               [ "86", "scripts.perf.cfc_construction.MixinTarget" ],
               [ "300", "scripts.perf.cfc_construction.MixinTarget300" ] ] ) {
	e = bench( row[2], ITER );
	writeOutput( row[1] & " methods: " & numberFormat( e, "99.999" ) & " us ("
		& numberFormat( e * 1000 / int( row[1] ), "999.9" ) & " ns/method)" & chr(10) );
}
</cfscript>
