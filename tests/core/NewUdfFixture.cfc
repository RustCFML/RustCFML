// Fixture for tests/core/test_new_udf_dispatch_and_null_call.cfm.
//
// `new` is a SOFT keyword on Lucee/Adobe CF/BoxLang: it introduces the
// `new Foo()` operator, but it is equally legal as a function name, and a
// BARE sibling call to it — `new (argumentCollection = {...})` with no
// component path after the keyword — dispatches to the UDF. Wheels' model
// creation depends on exactly this shape (vendor/wheels/model/create.cfc:32):
//
//     local.rv = new (argumentCollection = arguments);
//     local.rv.save(...);
component {

	// The sibling method literally named `new` (the Wheels model shape).
	public any function new(struct properties = {}) {
		return {
			marker = "NEW-RAN",
			props  = arguments.properties
		};
	}

	// The Wheels create() shape: a BARE call to the sibling UDF named `new`,
	// forwarding arguments via argumentCollection.
	public any function createViaBare() {
		return new (argumentCollection = {properties = {a = 1}});
	}

	// Control: the same call, explicitly this-qualified (works on both engines).
	public any function createViaThis() {
		return this.new(argumentCollection = {properties = {a = 1}});
	}

	// Null source for the second half of the test: a function that returns
	// nothing yields a null receiver, independent of the bare-new gap above.
	public any function giveNothing() {
		return;
	}

	// Real receiver for the method-call-on-null control.
	public string function save(numeric a = 0) {
		return "SAVED:" & arguments.a;
	}

}
