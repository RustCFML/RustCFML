component {
	variables.fmCompVar = "cv";
	this.fmPublic = "pub";

	public struct function closureFromInclude() {
		var fmMethodLocal = "before";
		include "p6_closure.cfm";
		fmMethodLocal = "after";
		return fmClosure();
	}

}
