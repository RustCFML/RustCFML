<cfscript>
suiteBegin( "cbehcache: CbEhCacheService / org.ehcache.Cache shim (RustCFML)" );
// The Java objects the cbehcache CacheBox provider drives. Semantics follow the
// jar's CbEhCacheService: timeouts in minutes, maxObjects caps a heap cache,
// disk storage is serialised and (persistent) survives a new manager.
_ehDir = getTempDirectory() & "rustcfml-ehcache-test-" & createUUID();
_ehMgr = createObject( "java", "org.pixl8.cbehcache.CbEhCacheService", "org.pixl8.cbehcache" ).init( _ehDir );
assert( "a new manager is available", _ehMgr.getStatus(), "AVAILABLE" );

heap = _ehMgr.createCache( "heap", { storage = "heap", maxObjects = 3, objectDefaultTimeout = 60, valueClass = "java.lang.Object" } );
assertThrows( "a second cache with the same name", function(){ _ehMgr.createCache( "heap", {} ); } );
heap.put( "a", { v = 1 } ); heap.put( "b", 2 ); heap.put( "c", [ 3 ] );
assert( "get", heap.get( "a" ).v, 1 );
assertTrue( "containsKey", heap.containsKey( "b" ) );
assertTrue( "a missing key reads null", isNull( heap.get( "zz" ) ) );
heap.put( "d", 4 ); // over maxObjects: evicts the least recently used (b)
keys = []; it = heap.iterator(); while ( it.hasNext() ) arrayAppend( keys, it.next().getKey() );
assert( "LRU eviction at maxObjects", arrayToList( keys ), "c,a,d" );
ref = { n = 1 }; heap.put( "ref", ref ); ref.n = 2;
assert( "the heap tier holds values by reference", heap.get( "ref" ).n, 2 );
heap.remove( "ref" ); assertFalse( "remove", heap.containsKey( "ref" ) );
st = _ehMgr.getStats( "heap" );
// gets: a (hit), zz (miss), ref (hit)
assert( "stats: hits", st.getCacheHits(), 2 );
assert( "stats: misses", st.getCacheMisses(), 1 );
assertTrue( "stats: evictions", st.getCacheEvictions() >= 1 );
known = st.getKnownStatistics();
assertTrue( "known statistics carry a MappingCount", structKeyExists( known, "OnHeap:MappingCount" ) );
// putting `ref` (a 4th entry) evicted c; removing ref leaves a, d
assert( "MappingCount.value()", known[ "OnHeap:MappingCount" ].value(), 2 );
st.clear(); assert( "stats clear", _ehMgr.getStats( "heap" ).getCacheHits(), 0 );
heap.clear(); assertFalse( "clear", heap.containsKey( "a" ) );

// A cache handle held one level down: `put` (a mutating method name) returns
// null, which must not be written back over the holder's reference.
holder = { c = heap };
holder.c.put( "deep", "value" );
assertFalse( "holder still holds the cache after put()", isNull( holder.c ) );
assert( "and the value is in it", holder.c.get( "deep" ), "value" );

typed = _ehMgr.createCache( "typed", { storage = "heap", valueClass = "struct" } );
assertThrows( "valueClass is enforced", function(){ typed.put( "x", "not a struct" ); } );

disk = _ehMgr.createCache( "disk", { storage = "disk", persistent = true, valueClass = "struct", maxSizeInMb = 5 } );
src = { html = "<p>one</p>" }; disk.put( "/p/1", src ); src.html = "changed";
assert( "the disk tier stores a copy", disk.get( "/p/1" ).html, "<p>one</p>" );
assertThrows( "the disk tier refuses a non-data value", function(){ disk.put( "/p/f", { fn = function(){} } ); } );
_ehMgr.close();
assert( "a closed manager", _ehMgr.getStatus(), "UNINITIALIZED" );
assertThrows( "a closed manager's cache is unusable", function(){ disk.get( "/p/1" ); } );
_ehMgr2 = createObject( "java", "org.pixl8.cbehcache.CbEhCacheService" ).init( _ehDir );
disk2 = _ehMgr2.createCache( "disk", { storage = "disk", persistent = true, valueClass = "struct", maxSizeInMb = 5 } );
assert( "a persistent disk cache survives a new manager", disk2.get( "/p/1" ).html, "<p>one</p>" );
_ehMgr2.close();
directoryDelete( _ehDir, true );
suiteEnd();
</cfscript>
