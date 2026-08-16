component {
	// Pseudo-constructor with an OBSERVABLE side effect: instantiation must run
	// this once per instance, metadata derivation must not be able to stand in
	// for it. `request` scope persists across includes within one execution.
	if ( !structKeyExists( request, "metacacheCtorRuns" ) ) {
		request.metacacheCtorRuns = 0;
	}
	request.metacacheCtorRuns = request.metacacheCtorRuns + 1;
	this.stamp = request.metacacheCtorRuns;

	public numeric function getStamp() {
		return this.stamp;
	}

	public string function sharedMethod() {
		return "shared";
	}
}
