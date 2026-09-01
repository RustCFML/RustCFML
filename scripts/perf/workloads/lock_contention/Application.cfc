component {

	this.name = "rustcfml_lock_bench";
	this.sessionManagement = false;

	// An Application.cfc must exist for the application scope to persist across
	// requests — without one the scope is per-request, and a lock benchmark that
	// silently measured nothing shared would report a flattering, meaningless
	// number. (Same trap as the extension memoiser: verify with plain CFML first.)
	public boolean function onApplicationStart() {
		application.hits = 0;
		return true;
	}

}
