component {
	// A component split across files: the included template declares methods
	// of THIS class (GH #463). The include runs for every instance, as on Lucee.
	variables.secret = "s3";
	include "include_methods_part.cfm";
	public string function inlineOne() { return "inline"; }
	this.pubData = 1;
	this.nullData = nullValue();
}
