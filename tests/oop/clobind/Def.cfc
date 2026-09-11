component {
	variables.who = "def";
	this.pub = "def-this";
	variables.counter = 0;
	function mk() {
		var loc = "loc";
		return function( x ) {
			variables.counter++;
			return "vars=" & variables.who & " this=" & this.pub & " unscoped=" & who & " loc=" & loc & " cnt=" & variables.counter & " x=" & x;
		};
	}
	function mkArrow() { return ( x ) => "vars=" & variables.who & " this=" & this.pub & " unscoped=" & who; }
	function mkWriter() { return function() { variables.written = "by-closure"; unscopedWrite = "unscoped-by-closure"; return "ok"; }; }
	function mkNested() { return function() { var inner = function() { return "vars=" & variables.who & " this=" & this.pub; }; return inner(); }; }
	function mkUdfRef() { return plainUdf; }
	function plainUdf() { return "vars=" & variables.who & " this=" & this.pub; }
	function getWritten() { return structKeyExists( variables, "written" ) ? variables.written : "none"; }
	function getUnscoped() { return structKeyExists( variables, "unscopedWrite" ) ? variables.unscopedWrite : "none"; }
	function getCounter() { return variables.counter; }
}
