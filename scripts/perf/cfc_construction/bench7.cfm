<cfscript>
// GH #425: subtractive ladder decomposing the FIXED cost of construction.
// Each arm adds one step; the delta between adjacent arms is that step's cost.
ITER = int( url.iter ?: 20000 );
WARM = int( url.warm ?: 20000 );
P = "scripts.perf.cfc_construction.";

function nullFn( numeric level = 0 ) { return 1; }

function timeIt( required any body, iters ) {
	var i = 0;
	for ( i = 1; i <= WARM; i++ ) { body(); }
	var t = getTickCount( "nano" );
	for ( i = 1; i <= iters; i++ ) { body(); }
	return ( getTickCount( "nano" ) - t ) / iters / 1000;
}

arms = [
  [ "a. bare UDF call (no CFC at all)",        function() { nullFn( 0 ); } ],
  [ "b. createObject 0 methods, no init",      function() { createObject( "component", P & "ladder.L0Empty" ); } ],
  [ "c. createObject 1 method,  no init",      function() { createObject( "component", P & "ladder.L1Init" ); } ],
  [ "d. c + .init(0)",                         function() { createObject( "component", P & "ladder.L1Init" ).init( 0 ); } ],
  [ "e. d + pseudo-ctor variables.someState",  function() { createObject( "component", P & "ladder.L2State" ).init( 0 ); } ],
  [ "f. createObject 20 methods, no init",     function() { createObject( "component", P & "MixinTarget20" ); } ],
  [ "g. f + .init(0)   <- the GH425 number",    function() { createObject( "component", P & "MixinTarget20" ).init( 0 ); } ],
  [ "h. new 20 methods (new kw + init)",       function() { new "#P#MixinTarget20"( 0 ); } ]
];
prev = 0;
for ( a in arms ) {
	e = timeIt( a[2], ITER );
	writeOutput( ljustify( a[1], 42 ) & numberFormat( e, "99.999" ) & " us   delta "
		& numberFormat( e - prev, "+99.999" ) & chr(10) );
	prev = e;
}
</cfscript>
