// Target for the invoke() argumentCollection-spread test.
component {
	public string function target(string name = "DEFAULT", string path = "P0") {
		return "name=" & arguments.name & " path=" & arguments.path;
	}

	// Reports which keys actually landed in the arguments scope.
	public string function argKeyList() {
		return structKeyList(arguments);
	}

	// True only if the literal key "argumentCollection" leaked through unspread.
	public boolean function hasLiteralArgumentCollectionKey() {
		return structKeyExists(arguments, "argumentCollection");
	}
}
