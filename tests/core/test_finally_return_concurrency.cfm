<cfscript>
// A `return` inside `lock {}` or `try {} finally {}` stashes its value while the
// finally body runs. The temp used to be undeclared, so in a component method it
// landed in the instance's shared `variables` scope: concurrent calls on one
// instance (a singleton service, e.g. CacheBox's CacheFactory.cacheExists /
// getCache, both `return` inside a readonly lock) got each other's values.

suiteBegin( "Return inside finally: the temp is a function local" );

fixture = new FinallyReturnFixture();
fixture.existsLocked( "a" );
fixture.labelTryFinally( "a" );
leaked = fixture.variablesKeys().filter( function( k ) { return k contains "__cf_finally"; } );
assert( "no finally temp in the instance's variables scope", ArrayLen( leaked ), 0 );

suiteEnd();

suiteBegin( "Return inside finally: concurrent calls on one instance" );

shared = new FinallyReturnFixture();
tasks = [];
for ( t = 1; t <= 16; t++ ) {
	tasks.append( runAsync( function() {
		var bad = 0;
		for ( var i = 1; i <= 100; i++ ) {
			switch ( i mod 3 ) {
				case 0: if ( shared.existsLocked( "a" ) !== true ) bad++; break;
				case 1: if ( shared.labelLocked( "a" ) != "label-a" ) bad++; break;
				default: if ( shared.labelTryFinally( "a" ) != "label-a" ) bad++;
			}
		}
		return bad;
	} ) );
}
wrong = 0;
for ( f in tasks ) {
	wrong += f.get();
}
assert( "every call returns its own value", wrong, 0 );

suiteEnd();
</cfscript>
