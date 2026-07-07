component {

	// Fixture for core/test_defaulted_param_variables_clobber.cfm.
	//
	// A component method (helper) shares its name with a defaulted parameter of
	// another method (useHelper's `helper` arg). In classic localmode a bare
	// assignment in a CFC method writes to `variables`; the default-value
	// preamble for an OMITTED defaulted param must NOT be treated as such a bare
	// write, or it clobbers `variables.helper` (the method) permanently.

	public string function helper() {
		return "HELPER_METHOD";
	}

	// `helper` is a defaulted param that shadows the method name for this call.
	public string function useHelper( string helper = "defaulted" ) {
		// The param is visible as data here …
		return arguments.helper;
	}

	// After useHelper() ran with its default applied, the method must survive.
	public string function callHelperBare() {
		return helper();
	}
}
