// Fixture: a component with a method literally named `new`. On Lucee/Adobe CF/
// BoxLang `new` is a SOFT keyword — it introduces the `new Foo()` operator, but
// is equally legal as a function name, and calling it via `this.new()` works.
// On RustCFML 0.37.0 `new` is a HARD reserved keyword, so `function new(){...}`
// fails to parse ("Expected identifier, found New") and the component degrades
// to a non-object.
//
// Wheels' core object-creation API depends on this: `model("User").new()` is
// backed by `public any function new(...)` in vendor/wheels/model/create.cfc.
component {
	public string function new() {
		return "made";
	}
	public string function probe() {
		return this.new();
	}
}
