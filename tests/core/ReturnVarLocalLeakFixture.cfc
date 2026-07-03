// Fixture: cfinvoke's returnVariable="local.rv" must stay FRAME-PRIVATE. A
// helper method that captures a call result into `local.rv` (the Wheels
// `$invoke` idiom) must NOT leak that `rv` into the CALLER's `local` scope.
// Wheels' $saveAssociations loops calling each child save() via $invoke
// (returnVariable="local.rv"); a child's successful save leaking local.rv=true
// over the parent loop's running local.rv=false broke nested-save rollback.
component {
	// Loops calling wrapInvoke; tracks its OWN local.rv across iterations. A leak
	// from wrapInvoke's cfinvoke would clobber outerRv.
	public string function outer() {
		local.outerRv = "OUTER";
		local.log = [];
		for (local.i = 1; local.i <= 3; local.i++) {
			// running accumulator like $saveAssociations' `if (rv) rv = saveResult`
			if (local.i == 2) { local.accum = false; } else { local.accum = local.accum ?: true; }
			local.cr = wrapInvoke(local.i);
			arrayAppend(local.log, "i=" & local.i & ":outerRv=" & local.outerRv & ",accum=" & local.accum & ",cr=" & local.cr);
		}
		return arrayToList(local.log, " | ");
	}

	// The Wheels $invoke shape: cfinvoke with dynamic returnVariable="local.rv".
	public any function wrapInvoke(n) {
		var args = { returnVariable = "local.rv", component = this, method = "inner" };
		cfinvoke(attributeCollection = "#args#");
		if (structKeyExists(local, "rv")) { return local.rv; }
		return "NO_RV";
	}

	public boolean function inner() { return true; }
}
