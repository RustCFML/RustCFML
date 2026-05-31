// Fixture: a function whose parameters are named with the soft keywords
// `extends` and `implements`. On Lucee/Adobe CF/BoxLang these are legal
// parameter names (and reachable via the arguments scope). On RustCFML 0.37.0
// they are hard reserved keywords, so the parameter declaration fails to parse
// ("Expected RParen, found Extends") and the component degrades to a non-object.
//
// Mirrors vendor/wheels/wheelstest/system/mockutils/MockGenerator.cfc:
//   function generateClass( string extends="", string implements="" ) { ... }
component {
	public string function gen(string extends = "", string implements = "") {
		return arguments.extends & "/" & arguments.implements;
	}
	public string function probe() {
		return gen("a", "b");
	}
}
