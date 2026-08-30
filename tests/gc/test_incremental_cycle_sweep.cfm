<cfscript>
// The mid-request cycle sweep (`cycle_gc::collect_incremental`) is what keeps a
// component-heavy request from holding every instance it ever built: a CFC is
// inherently cyclic, so without it 100k constructions retained 1.8 GB.
//
// The danger of collecting mid-request is OVER-collection — reclaiming
// something still reachable. These specs build a lot of instances, keep them
// all live, and read every one back afterwards: any over-collection shows up as
// wrong data, a broken method dispatch, or a throw.
//
// NB whether a sweep actually FIRES here depends on the default threshold, which
// CFML cannot set. The sweep's own semantics (reclaims unreachable cycles, never
// collects a live one, re-registers survivors, backs the budget off with the
// live set) are pinned directly in Rust — `cycle_gc::incremental_tests`. This
// file is the end-to-end correctness half: heavy real construction under
// whatever sweeping the build does.
suiteBegin( "Incremental cycle sweep does not collect live objects" );

held = [];
for ( i = 1; i <= 15000; i++ ) {
	arrayAppend( held, new gc.SweepModel( "tag#i#" ) );
}

assert( "every instance retained", arrayLen( held ), 15000 );

// Read back AFTER the sweeps have run: private state, public state, dispatch.
badVars = 0; badThis = 0; badCall = 0;
for ( i = 1; i <= 15000; i++ ) {
	if ( held[ i ].readTag()     != "tag#i#" ) { badVars++; }
	if ( held[ i ].readThisTag() != "tag#i#" ) { badThis++; }
	if ( held[ i ].echo( i )     != i        ) { badCall++; }
}
assert( "variables-scope state survives the sweep", badVars, 0 );
assert( "this-scope state survives the sweep",      badThis, 0 );
assert( "method dispatch survives the sweep",       badCall, 0 );

// Interleaving live and discarded instances must not confuse the sweep: the
// discarded ones are exactly what it is meant to reclaim.
kept = [];
for ( i = 1; i <= 8000; i++ ) {
	throwaway = new gc.SweepModel( "junk" );      // becomes garbage immediately
	if ( i % 100 == 0 ) { arrayAppend( kept, new gc.SweepModel( "keep#i#" ) ); }
}
assert( "interleaved keeps retained", arrayLen( kept ), 80 );
bad = 0;
for ( i = 1; i <= arrayLen( kept ); i++ ) {
	if ( kept[ i ].readTag() != "keep#( i * 100 )#" ) { bad++; }
}
assert( "interleaved keeps intact after sweeps", bad, 0 );

suiteEnd();
</cfscript>
