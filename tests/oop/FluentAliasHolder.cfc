component {
	// Captures a reference to the target BEFORE it is stubbed — mirrors ColdBox's
	// `Builder.init( mockInjector )` (GH ##263). Every later chained stub on the
	// target must remain visible through this held alias.
	function init( any target ){ if ( !isNull( arguments.target ) ) { variables.target = arguments.target; } return this; }
	function callM1(){ return variables.target.m1(); }
	function callM2( required key ){ return variables.target.m2( arguments.key ); }
}
