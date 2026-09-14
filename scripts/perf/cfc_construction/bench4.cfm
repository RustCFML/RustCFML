<cfscript>
// Is StructAppend expensive because the entries are FUNCTIONS, or just because
// there are N of them? Same count, different value types.
ITER = int( url.iter ?: 3000 );
WARM = int( url.warm ?: 3000 );
N    = int( url.n ?: 300 );

mp = createObject( "component", "scripts.perf.cfc_construction.MixinTarget300" );
funcs = {};
strs  = {};
i = 0;
for ( f in getMetaData( mp ).functions ) {
	if ( structCount( funcs ) GE N ) { break; }
	funcs[ f.name ] = mp[ f.name ];
	strs[ f.name ]  = "a string standing in for a method entry";
}
structs = {};
for ( k in funcs ) { structs[ k ] = { a = 1, b = "two" }; }

function bench( label, src, iters ) {
	var i = 0;
	for ( i = 1; i <= WARM; i++ ) { var d = {}; structAppend( d, src ); }
	var t = getTickCount( "nano" );
	for ( i = 1; i <= iters; i++ ) { var d = {}; structAppend( d, src ); }
	var e = ( getTickCount( "nano" ) - t ) / iters / 1000;
	writeOutput( label & " = " & numberFormat( e, "99.999" ) & " us  ("
		& numberFormat( e * 1000 / structCount( src ), "999.9" ) & " ns/entry)" & chr(10) );
	return e;
}

writeOutput( "entries = " & structCount( funcs ) & chr(10) );
bench( "structAppend of FUNCTION entries", funcs,   ITER );
bench( "structAppend of STRING entries  ", strs,    ITER );
bench( "structAppend of STRUCT entries  ", structs, ITER );
</cfscript>
