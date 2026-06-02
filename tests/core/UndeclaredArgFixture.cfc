// Fixture for the "undeclared named arguments keep their names" behavior.
// `capture()` declares only `a`, but is called with extra named args (b, c).
// On Lucee/Adobe CF/BoxLang those extras remain reachable by name in the
// `arguments` scope; on RustCFML they are stored positionally (numeric keys),
// so StructKeyExists(arguments, "b") is false.
//
// Wheels relies on this in vendor/wheels/Global.cfc $createObjectFromRoot: it
// declares path/fileName/method, receives extra named args (e.g. pluginPath),
// then forwards the whole `arguments` struct to the target method.
component {
	public string function probe() {
		return capture(a = "A", b = "B", c = "C");
	}
	private string function capture(required string a) {
		return StructKeyExists(arguments, "b") && StructKeyExists(arguments, "c")
			? "a=" & arguments.a & ",b=" & arguments.b & ",c=" & arguments.c
			: "MISSING (keys=" & StructKeyList(arguments) & ")";
	}
}
