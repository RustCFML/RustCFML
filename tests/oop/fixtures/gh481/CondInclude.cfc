component {
	// Literal path, but the include only runs once the request says so — the
	// method set is per instance, not per class (GH 481, second report).
	if ( structKeyExists( request, "gh481Ready" ) ) {
		include "cond/helpers.cfm";
	}
	function gh481CallHelper() { return gh481HelperG(); }
}
