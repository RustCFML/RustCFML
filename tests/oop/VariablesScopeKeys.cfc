/**
 * Fixture for test_variables_scope_keys.cfm: what a method sees when it
 * enumerates its component's `variables` scope.
 */
component {
	this.pub = 1;
	variables.priv = 2;
	static { counter = 3; }

	function keyList() { return listSort( structKeyList( variables ), "textnocase" ); }
	function keyArray() { var a = structKeyArray( variables ); arraySort( a, "textnocase" ); return a.toList(); }
	function keyCount() { return structCount( variables ); }
	function forInKeys() { var out = []; for ( var k in variables ) out.append( k ); arraySort( out, "textnocase" ); return out.toList(); }
	function lastKey() { var a = structKeyArray( variables ); return a[ a.len() ]; }
	function thisFromLoop() { for ( var k in variables ) { if ( k == "this" ) return variables[ k ].pub; } return "none"; }
	function eachKeys() { var out = []; structEach( variables, function( k ){ out.append( k ); } ); return out.toList(); }
	function copyKeys() { return listSort( structKeyList( structCopy( variables ) ), "textnocase" ); }
	function jsonHasStatic() { return findNoCase( "__static", serializeJSON( variables ) ) > 0; }
	function staticStillWorks() { return static.counter; }
}
