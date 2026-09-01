/**
 * Fixture for tests/core/test_keyword_component_path_case.cfm — a component
 * whose NAME is a CFML keyword, in camel case. `new core.Public()` had to probe
 * `core/public.cfc` before the parser kept the source spelling (GH 381).
 */
component {
	function init() { return this; }
	function whoAmI() { return "core.Public"; }
}
