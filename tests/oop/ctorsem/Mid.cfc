component extends="Root" {
	request.ctorsemLog = listAppend( request.ctorsemLog, "Mid" );
	function init() { return this; }
	function midMethod() { return "mid-method"; }
}
