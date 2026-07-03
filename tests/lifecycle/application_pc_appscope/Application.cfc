component {
	// The pseudo-constructor runs BEFORE the real named application scope is
	// bound. Lucee gives it a live, writable working `application` scope: writes
	// succeed and immediate read-backs return the value (cross-engine verified),
	// but those writes are discarded once the named scope binds. Preside's
	// `_getDefaultStatelessUrlPatterns` (guard → set → return) relies on the
	// read-back working. Record everything into `request` so index.cfm can report.
	this.name = "pc-appscope-test";
	request.pc = {};

	// direct write + immediate read-back inside the PC
	application.pcVal = "written-in-pc";
	request.pc.readback = application.pcVal;                       // must NOT throw
	request.pc.ske      = structKeyExists( application, "pcVal" ); // must be true in the PC

	// the Preside guard-set-then-return shape
	request.pc.preside = _presideLike();

	function _presideLike() {
		if ( !structKeyExists( application, "_pat" ) ) {
			application._pat = "built";
		}
		return application._pat;
	}
}
