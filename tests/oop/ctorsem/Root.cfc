component {
	// Every level of the chain logs its pseudo-constructor run, in order.
	param name="request.ctorsemLog" default="";
	request.ctorsemLog = listAppend( request.ctorsemLog, "Root" );
	variables.fromRoot = "root";
	function init() { return this; }
	function rootMethod() { return "root-method"; }
}
