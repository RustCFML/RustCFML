component extends="AccessGateParent" {

	public string function childCallsInheritedPrivate() {
		return parentPriv();
	}

	public string function childCallsInheritedPrivateViaThis() {
		return this.parentPriv();
	}

	public string function childCallsInheritedPrivateViaSuper() {
		return super.parentPriv();
	}

	private string function childPriv() {
		return "childPriv";
	}
}
