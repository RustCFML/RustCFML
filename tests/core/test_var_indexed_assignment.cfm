<cfscript>
suiteBegin("var indexed assignment");

// Regression: `var m[ key ] = value` (var on a SUBSCRIPTED target) previously
// parsed as a bare `var m` re-declaration — it wiped the existing local struct
// and dropped the write. Preside's Renderer builds its whole view-mapping struct
// with `var mappings[ mapping ] = …` inside a loop, so the map came out empty and
// every front-end view resolved to /app/views and 404'd. `var m[k]=v` must write
// into the existing local `m` (Lucee parity).

// --- loop accumulation (the Preside idiom) ---
function buildLoop() {
	var m = {};
	for ( k in [ "a", "b", "c" ] ) {
		var m[ k ] = "val_" & k;
	}
	return m;
}
r = buildLoop();
assert( "loop var[key] accumulates all keys", structCount( r ), 3 );
assert( "loop var[key] value a", r.a, "val_a" );
assert( "loop var[key] value c", r.c, "val_c" );

// --- var[key] does NOT wipe a prior plain write ---
function buildMixed() {
	var m = {};
	m[ "x" ] = "plain";
	var m[ "y" ] = "varsub";
	return m;
}
r2 = buildMixed();
assert( "var[key] preserves prior key", structCount( r2 ), 2 );
assertTrue( "prior plain key survives", structKeyExists( r2, "x" ) );
assertTrue( "var[key] adds its key", structKeyExists( r2, "y" ) );
assert( "var[key] value", r2.y, "varsub" );

// --- dynamic key expression ---
function buildDynamic() {
	var m = {};
	var prefix = "col_";
	for ( i = 1; i <= 3; i++ ) {
		var m[ prefix & i ] = i * 10;
	}
	return m;
}
r3 = buildDynamic();
assert( "dynamic-key count", structCount( r3 ), 3 );
assert( "dynamic-key value", r3[ "col_2" ], 20 );

suiteEnd();
</cfscript>
