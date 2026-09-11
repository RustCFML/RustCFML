component {
	variables.who = "tgt";
	this.pub = "tgt-this";
	variables.counter = 100;
	function inject( n, f ) { this[ n ] = f; variables[ n ] = f; return this; }
	function callBare( x ) { return clo( x ); }
	function callThis( x ) { return this.clo( x ); }
	function callVars( x ) { return variables.clo( x ); }
	function callArrowBare() { return arr(); }
	function callWriter() { return wr(); }
	function callNested() { return nest(); }
	function callUdfBare() { return udf(); }
	function getWritten() { return structKeyExists( variables, "written" ) ? variables.written : "none"; }
	function getUnscoped() { return structKeyExists( variables, "unscopedWrite" ) ? variables.unscopedWrite : "none"; }
	function getCounter() { return variables.counter; }
	function callStructHeld( x ) { var s = { f: clo }; return s.f( x ); }
}
