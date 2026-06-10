component {

	// Undeclared callee: must get a fresh, empty `local` regardless of the
	// caller's same-named local.rv.
	public struct function innerProbe() {
		var r = {
			seesRv   = StructKeyExists(local, "rv"),
			rvIsNull = isNull(local.rv),
			leaked   = ""
		};
		if (r.seesRv) r.leaked = toString(local.rv);
		return r;
	}

	public struct function outerProbe() {
		local.rv = false;
		return innerProbe();
	}

	// The Wheels $callback() default-true tail: returns true unless THIS
	// body set local.rv (it never does here).
	public boolean function callbackTail() {
		if (!StructKeyExists(local, "rv")) {
			local.rv = true;
		}
		return local.rv;
	}

	public boolean function invokeWithAccumulator() {
		local.rv = false; // caller's own working flag, same conventional name
		return callbackTail();
	}

}
