component {
	// Fresh application per test run: the runner passes a unique token so a
	// warm server re-running the suite still exercises a cold app start.
	this.name = "appstartgate_" & (url.app ?: "default");
	this.sessionmanagement = false;

	public boolean function onApplicationStart() {
		application.runs = (application.runs ?: 0) + 1;
		application.phase = "starting";
		sleep(1500);
		application.ready = true;
		return true;
	}

	public boolean function onRequest(required string targetPage) output=true {
		if ((url.op ?: "") == "runs") {
			writeOutput("runs=" & (application.runs ?: 0));
		} else {
			writeOutput("ready=" & (application.ready ?: "MISSING"));
		}
		return true;
	}
}
