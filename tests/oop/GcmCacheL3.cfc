/**
 * Fixture for tests/oop/test_gcm_path_metadata_cache.cfm — top of a 3-level
 * inheritance chain (L1 extends L2 extends L3).
 */
component {
	property name="l3prop" type="string" default="three";

	public string function l3One( required string a ) { return "l3One"; }
	public numeric function l3Two() { return 3; }
}
