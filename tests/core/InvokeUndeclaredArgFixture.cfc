component {
	public string function probe(required string a) {
		var hasB = StructKeyExists(arguments, "b");
		var hasC = StructKeyExists(arguments, "c");
		if (hasB && hasC) {
			return "a=" & arguments.a & ",b=" & arguments.b & ",c=" & arguments.c;
		}
		return "MISSING (keys=" & StructKeyList(arguments) & ")";
	}
}
