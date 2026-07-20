component {
	public string function probe() {
		return capture(a = "A", b = "B", c = "C");
	}

	private string function capture(required string a) {
		return StructKeyExists(arguments, "b") && StructKeyExists(arguments, "c")
			? "a=" & arguments.a & ",b=" & arguments.b & ",c=" & arguments.c
			: "MISSING (keys=" & StructKeyList(arguments) & ")";
	}

	// ColdBox preHandler idiom: `prc`/`rc` are passed as named args but the
	// method does NOT declare them, so they live in the arguments scope (not
	// frame locals). An UNSCOPED compound write `prc.x = v` must resolve the
	// base up the cascade (local -> arguments -> variables) and mutate the
	// arguments-scope struct in place, so the caller (action/view) sees it.
	// Previously the write forked a phantom `local.prc`, discarded on return —
	// which left Preside's LinkPicker.preHandler unable to populate prc.linkTypes
	// (empty link-type menu). Returns void so the assertion checks the caller's
	// own by-reference struct.
	public void function preHandlerLike( event, action ) {
		prc.injectedSingle = "from-prehandler";
	}

	// Multi-level (nested) member-write on the same undeclared-arg struct. Routes
	// through store_runtime_path rather than StoreLocalProperty; the modified root
	// must still commit back to the arguments scope (visible to the caller), not
	// the component variables scope or a phantom local.
	public void function preHandlerLikeNested( event, action ) {
		prc.nested.deep = "from-prehandler-nested";
	}
}
