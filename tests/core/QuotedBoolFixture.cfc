// Control fixture: boolean attribute value written as a QUOTED string. RustCFML
// already parses this — pairs with UnquotedBoolFixture to isolate the gap to the
// unquoted boolean keyword in the value position (not the `output` attribute).
component output="false" {
	public string function ping() {
		return "pong";
	}
}
