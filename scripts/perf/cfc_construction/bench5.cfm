<cfscript>
// The three mixin appends write INTO a component (variables scope and `this`),
// not into a plain struct. Is that the expensive part?
ITER = int( url.iter ?: 3000 );
WARM = int( url.warm ?: 3000 );

mp = createObject( "component", "scripts.perf.cfc_construction.MixinProvider" );
mixins = {};
for ( f in getMetaData( mp ).functions ) { mixins[ f.name ] = mp[ f.name ]; }

function bench( label, fn, iters ) {
	var i = 0;
	for ( i = 1; i <= WARM; i++ ) { fn(); }
	var t = getTickCount( "nano" );
	for ( i = 1; i <= iters; i++ ) { fn(); }
	var e = ( getTickCount( "nano" ) - t ) / iters / 1000;
	writeOutput( label & " = " & numberFormat( e, "99.999" ) & " us  ("
		& numberFormat( e * 1000 / structCount( mixins ), "9999.9" ) & " ns/entry)" & chr(10) );
	return e;
}

writeOutput( "appending " & structCount( mixins ) & " mixins" & chr(10) );

// A: into a fresh plain struct.
bench( "A into a plain struct        ", function() {
	var d = {};
	structAppend( d, mixins, true );
	return d;
}, ITER );

// B: into a plain struct that already holds 300 entries (size, not kind).
big = {};
for ( n = 1; n <= 300; n++ ) { big[ "k" & n ] = n; }
bench( "B into a 300-entry struct    ", function() {
	var d = structCopy( big );
	structAppend( d, mixins, true );
	return d;
}, ITER );

// C: into a 300-method component's `this`.
bench( "C into a component's THIS    ", function() {
	var o = createObject( "component", "scripts.perf.cfc_construction.MixinTarget300" ).init( 0 );
	structAppend( o, mixins, true );
	return o;
}, ITER );

// D: construction alone, to subtract from C.
bench( "D construction alone         ", function() {
	return createObject( "component", "scripts.perf.cfc_construction.MixinTarget300" ).init( 0 );
}, ITER );
</cfscript>
