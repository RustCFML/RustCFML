<cfscript>
// cacheGetProperties() — shape verified against a live Lucee 7:
//   * no-arg  → always an empty array (Lucee reports only Administrator-defined
//     default caches; a plain app has none).
//   * named   → a one-element array with that cache's live runtime stats:
//     [{ hit_count, miss_count, outOfMemoryHandling }] (RamCache's exact shape).
// RustCFML backs cache* with one always-on in-memory store, so the named form
// returns that store's REAL hit/miss counters. This unblocks ColdBox's CacheBox
// CFProvider/LuceeProvider, which call the no-arg form during cache startup.
suiteBegin( "cacheGetProperties" );

// No-arg → empty array (Lucee parity).
p = cacheGetProperties();
assertTrue( "no-arg returns array", isArray( p ) );
assert( "no-arg is empty", arrayLen( p ), 0 );

// Named → one-element array of stats with the exact Lucee keys.
named = cacheGetProperties( "object" );
assertTrue( "named returns array", isArray( named ) );
assert( "named has one entry", arrayLen( named ), 1 );
stats = named[ 1 ];
assertTrue( "has hit_count", structKeyExists( stats, "hit_count" ) );
assertTrue( "has miss_count", structKeyExists( stats, "miss_count" ) );
assertTrue( "has outOfMemoryHandling", structKeyExists( stats, "outOfMemoryHandling" ) );
assertFalse( "outOfMemoryHandling is false", stats.outOfMemoryHandling );

// Counters are REAL: a hit and a miss move the numbers.
before = cacheGetProperties( "object" )[ 1 ];
cachePut( "cgp_k", "v" );
cacheGet( "cgp_k" );        // hit
cacheGet( "cgp_absent" );   // miss
after = cacheGetProperties( "object" )[ 1 ];
assertTrue( "hit_count increased", after.hit_count > before.hit_count );
assertTrue( "miss_count increased", after.miss_count > before.miss_count );

suiteEnd();
</cfscript>
