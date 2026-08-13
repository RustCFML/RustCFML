/**
 * Fixture for tests/stdlib/test_duplicate_deepcopy_flag.cfm.
 *
 * Deliberately minimal: a single mutable `this` field, so a test can tell
 * whether duplicate() handed back the SAME component instance (a write through
 * the copy is visible on the source) or a copy of it.
 */
component {

	this.marker = "orig";

	function setMarker( required string m ) {
		this.marker = arguments.m;
	}

	function getMarker() {
		return this.marker;
	}

}
