component {
	public any function init() { return this; }
	public string function sign( required string input ) { return "signed:" & arguments.input; }
}
