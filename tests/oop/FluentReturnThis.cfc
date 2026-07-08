component {

	// Real methods that a stub is expected to override in place.
	function m1(){ return "REAL-M1"; }
	function m2( required key ){ return "REAL-M2"; }

	// Fluent stub-installer: injects a member (value OR function) onto `this`
	// and returns `this`, mirroring MockBox's `$()` (structDelete + inject +
	// `return this`). The 2nd-and-later installs in a chain used to be dropped
	// because the write-back's chained-CFC identity guard could not tell the
	// returned `this` (a value-semantics copy on a fresh Arc) from a foreign CFC.
	function stub( required name, any value = "" ){
		this[ arguments.name ] = arguments.value;
		return this;
	}

	// Plain fluent setter (value member) — the simplest form of the chain.
	function set( required k, required v ){
		this[ arguments.k ] = arguments.v;
		return this;
	}

	// No-op fluent step — returns `this` unchanged. Used to reproduce GH ##261:
	// a chain whose FIRST step mutates nothing, then injects a closure member.
	function noop(){ return this; }

	// Returns a DIFFERENT CFC — the case the identity guard MUST keep skipping.
	function getDep(){
		if ( isNull( variables.dep ) ) { variables.dep = new FluentReturnThis(); }
		return variables.dep;
	}
	function whoAmI(){ return "root"; }
}
