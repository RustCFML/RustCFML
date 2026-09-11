component {
	function inject( n, f ) { this[ n ] = f; variables[ n ] = f; return this; }
	function callBare( x ) { var callerLocal = "leak"; return probe(); }
	function callThis( x ) { var callerLocal = "leak"; return this.probe(); }
	function callStruct( x ) { var callerLocal = "leak"; var s = { f: probe }; return s.f(); }
}
