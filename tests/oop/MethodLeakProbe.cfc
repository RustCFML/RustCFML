/**
 * GH #360 — a CFC's method names must not become ambient bare names for every
 * template that runs later in the same request.
 */
component {
	public string function aVeryUniqueMethodName360() {
		return "ran";
	}
	public string function callsItsOwnSibling360() {
		return aVeryUniqueMethodName360();
	}
}
