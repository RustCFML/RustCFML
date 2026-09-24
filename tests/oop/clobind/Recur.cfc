component {
	variables.who = "recur";
	public array function recurse() {
		var out = [];
		var walk = function( depth ) {
			out.append( depth & ":" & _priv() & ":" & variables.who & ":" & this.tag() );
			if ( depth < 3 ) {
				walk( depth + 1 );
			}
		};
		walk( 1 );
		return out;
	}
	public string function sibling() {
		var inner = function() { return _priv() & "/" & variables.who; };
		var outer = function() { return inner(); };
		return outer();
	}
	public any function mkRecursive() {
		var out = [];
		var walk = function( depth ) {
			out.append( _priv() & ":" & variables.who );
			if ( depth < 2 ) {
				walk( depth + 1 );
			}
			return out;
		};
		return walk;
	}
	public string function tag() { return "recur-this"; }
	private string function _priv() { return "priv"; }
}
