component {

	// Statement-form cfinvoke that uses the `component` attribute — the exact
	// shape in Wheels' Global.cfc::$cfinvoke(). `component` is a SOFT keyword on
	// Lucee/ACF/BoxLang, so it is a legal attribute name here and this PARSES;
	// the cfinvoke invokes InvokeTarget.getValue(). RustCFML treats `component`
	// as the HARD reserved CFC keyword, so this fails to PARSE ("Expected LBrace,
	// found Equal"), degrading the WHOLE component to a non-object at
	// instantiation. Kept in a fixture so the parse failure is contained (the
	// test instantiates it at runtime) and does not abort the run.
	//
	// The assertion is about PARSE-ability (the RustCFML gap), not the exact
	// returnVariable semantics of statement-form cfinvoke (which vary), so the
	// return is guarded: if the invoke captured a value we return it, otherwise
	// we return the same sentinel — either way a parsing engine yields INVOKED_OK.
	function callViaCfinvoke() {
		cfinvoke
		component = "InvokeTarget"
		method = "getValue"
		returnVariable = "local.rv";
		return structKeyExists(local, "rv") ? local.rv : "INVOKED_OK";
	}

}
