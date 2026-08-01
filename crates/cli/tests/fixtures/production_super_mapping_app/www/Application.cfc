component extends="moopa.application" {
	this.name = "productionSuperMapping";
	// The parent lives OUTSIDE the served webroot and is only reachable via
	// this per-application mapping — the GH #301 shape. The engine's early
	// (pre-this.mappings) probe of the extends target must not poison the
	// post-mappings resolution.
	this.mappings = { "/moopa": "../moopa" };

	public boolean function onApplicationStart() {
		super.onApplicationStart();
		return true;
	}

	public boolean function OnRequestStart(required string targetPage) {
		super.OnRequestStart(arguments.targetPage);
		return true;
	}
}
