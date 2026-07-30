component {
	// The pseudo-constructor runs BEFORE the real named application scope is
	// bound. Lucee gives it a live, writable `application` scope: writes succeed,
	// immediate read-backs return the value, and the scope PERSISTS ACROSS
	// REQUESTS (it is the pre-`this.name` default application scope) — so a
	// guard-once block set by one request's PC is still set for the next request's
	// PC. Those writes stay invisible to the page body, which sees the named
	// scope. All three properties verified cross-engine against Lucee 7.0.4.34.
	//
	// Preside's `_getDefaultStatelessUrlPatterns` (guard → set → return) relies on
	// the read-back; `Bootstrap._setupCustomTagPaths` relies on the cross-request
	// persistence (without it its recursive DirectoryList over every extension
	// re-ran on every request, ~190ms). Record into `request` so index.cfm reports.
	this.name = "pc-appscope-test";
	request.pc = {};

	// Did this PC inherit the PREVIOUS request's PC write? (false on the first
	// request, true on every warm one.)
	request.pc.sawPrev = structKeyExists( application, "pcVal" );

	// direct write + immediate read-back inside the PC
	application.pcVal = "written-in-pc";
	request.pc.readback = application.pcVal;                       // must NOT throw
	request.pc.ske      = structKeyExists( application, "pcVal" ); // must be true in the PC

	// the Preside guard-set-then-return shape. `guardRan` tells us whether the
	// expensive branch executed — it must run ONCE, not on every request.
	request.pc.guardRan = false;
	request.pc.preside  = _presideLike();

	function _presideLike() {
		if ( !structKeyExists( application, "_pat" ) ) {
			application._pat = "built";
			request.pc.guardRan = true;
		}
		return application._pat;
	}
}
