component {
	function init() { variables.instanceVar = "IVAR-" & createUUID(); return this; }
	function loadHelpers() {
		var methodLocal = "SECRET";
		include "mixinlib.cfm";
		return this;
	}
}
