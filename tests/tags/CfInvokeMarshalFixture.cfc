// Fixture for the cfinvoke call-form marshaling test. `hello` and `sibling`
// are plain probes; `invokeSibling` exercises the componentless call form
// FROM INSIDE a CFC method — the Wheels Global.cfc $invoke() shape when the
// target method already lives on the calling component. The try/catch turns
// an engine error into a comparable string so the suite reports a failed
// assertion instead of aborting.
component {

	public string function hello(string who = "world") {
		return "hello-" & arguments.who;
	}

	public string function sibling() {
		return "sibling-ok";
	}

	public string function invokeSibling() {
		try {
			cfinvoke(method = "sibling", returnVariable = "local.rv");
		} catch (any e) {
			return "THREW: " & e.message;
		}
		return StructKeyExists(local, "rv") ? local.rv : "RV-UNSET";
	}

}
