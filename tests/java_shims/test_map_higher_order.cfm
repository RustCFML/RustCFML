<cfscript>
// GH #239: java.util.Map-family shims (LinkedHashMap/TreeMap/ConcurrentHashMap)
// must support the struct higher-order member functions (each/map/filter/reduce)
// over their real entries — consistent with structCount()/structKeyList() which
// already see them. Previously the shim dispatch consumed the callback arg via
// mem::take and then fell through to the generic struct HOF with an empty arg
// list, so .each()/.map()/.reduce() silently iterated ZERO times (TestBox's
// CoverageReporter merge path stored lineData as a LinkedHashMap → downstream
// "Variable 'value' is undefined").
suiteBegin( "Java Shims: Map higher-order functions (GH ##239)" );

lhm = createObject( "java", "java.util.LinkedHashMap" ).init();
lhm.put( "1", 1 );
lhm.put( "2", 0 );

// baseline: struct fns already saw the entries (and hid the __markers)
assert( "structCount", structCount( lhm ), 2 );
assert( "structKeyList", structKeyList( lhm ), "1,2" );

// each iterates exactly the real entries — not the internal __java markers
count = 0;
keysSeen = "";
lhm.each( function( key, value ){ count++; keysSeen = listAppend( keysSeen, key ); } );
assert( "each visits every entry once", count, 2 );
assert( "each never leaks __markers", listSort( keysSeen, "text" ), "1,2" );

// map returns a struct of the same real keys with transformed values
m = lhm.map( function( k, v ){ return v + 10; } );
assert( "map keys", structKeyList( m ), "1,2" );
assert( "map value 1", m[ "1" ], 11 );
assert( "map value 2", m[ "2" ], 10 );
assertFalse( "map result has no __java_shim key", structKeyExists( m, "__java_shim" ) );

// reduce folds over the real values only
r = lhm.reduce( function( acc, k, v ){ return acc + v; }, 0 );
assert( "reduce sums values", r, 1 );

// filter keeps matching entries only
f = lhm.filter( function( k, v ){ return v gt 0; } );
assert( "filter keys", structKeyList( f ), "1" );

// get(key) still works after the fall-through arg-restore
assert( "get after HOFs", lhm.get( "1" ), 1 );

suiteEnd();
</cfscript>
