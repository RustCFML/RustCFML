component {
	function init( required svc ) {
		variables.svc = arguments.svc;
		return this;
	}
	function getSvc() {
		return variables.svc;
	}
}
