/**
 * Inheritance half of the GH #330 access-gate fixture: a private method DECLARED
 * on the parent, reached from a child method (legal) and from outside (refused).
 */
component {

	private string function parentPriv() {
		return "parentPriv";
	}

	public string function parentCallsPrivate() {
		return parentPriv();
	}

	public string function parentCallsPrivateViaThis() {
		return this.parentPriv();
	}
}
