component {
	public boolean function onApplicationStart() {
		application.bootMarker = "parent-boot";
		return true;
	}

	public boolean function OnRequestStart(required string targetPage) {
		request.parentMarker = "parent-request";
		return true;
	}
}
