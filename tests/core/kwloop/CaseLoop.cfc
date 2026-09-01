component {
	// The shape found in a Preside application service: `case` is a perfectly
	// ordinary variable name on Lucee/ACF, and reserved-word loop variables are
	// common enough in real code that the parser must not special-case them.
	public string function joinCases( required query cases ) {
		var out = "";
		for ( var case in arguments.cases ) {
			out = ListAppend( out, case.label );
		}
		return out;
	}
	public string function loopKeywords() {
		var out = "";
		for ( var switch in [ "a" ] ) { out &= switch; }
		for ( var default in [ "b" ] ) { out &= default; }
		for ( var new in [ "c" ] ) { out &= new; }
		for ( var case in [ "d" ] ) { out &= case; }
		for ( var package in [ "e" ] ) { out &= package; }
		// A reserved word that is also a LITERAL (`true`, `null`) or a statement
		// keyword (`return`) may name the loop variable, but reading it back by
		// that name yields the literal/keyword rather than the variable. Assert
		// only the words that read back as variables on BOTH engines.
		return out;
	}
}
