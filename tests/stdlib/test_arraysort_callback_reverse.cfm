<cfscript>
suiteBegin("stdlib: arraySort callback comparator semantics (GH ##233)");

// A UDF comparator that always returns -1 fully REVERSES the array on Lucee/ACF
// (TestBox uses exactly `arraySort(tree, (a,b)=>-1)` to reverse its beforeEach
// hook tree — a no-op here previously dropped the innermost nested beforeEach,
// GH #233). Verified byte-for-byte against Lucee 6.

a = [ 1, 2, 3, 4, 5 ];
arraySort( a, function( x, y ){ return -1; } );
assert("always-(-1) comparator reverses", a.toList(), "5,4,3,2,1");

b = [ 1, 2, 3, 4, 5 ];
arraySort( b, function( x, y ){ return 1; } );
assert("always-(+1) comparator is a no-op", b.toList(), "1,2,3,4,5");

c = [ 1, 2, 3 ];
arraySort( c, function( x, y ){ return 0; } );
assert("always-0 comparator is a no-op", c.toList(), "1,2,3");

// ordinary comparators still sort correctly and stably
asc = [ 3, 1, 4, 1, 5 ];
arraySort( asc, function( x, y ){ return x - y; } );
assert("ascending comparator", asc.toList(), "1,1,3,4,5");

desc = [ 3, 1, 4, 1, 5 ];
desc.sort( function( x, y ){ return y - x; } );
assert("descending via .sort() member", desc.toList(), "5,4,3,1,1");

suiteEnd();
</cfscript>
