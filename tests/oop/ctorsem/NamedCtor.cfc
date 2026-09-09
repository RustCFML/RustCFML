component name="NamedCtor" {
	param name="request.ctorsemNamedRuns" default=0;
	request.ctorsemNamedRuns++;
	this.x = 1;
	function init() { return this; }
}
