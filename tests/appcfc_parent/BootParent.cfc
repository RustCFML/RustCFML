component {
	// Fixture for test_appcfc_extends_parent_methods.cfm — the shape of Preside's
	// Bootstrap.cfc: an Application.cfc parent whose lifecycle methods call the
	// parent's PRIVATE helpers, with default-parameter expressions doing the same.
	public void function setupApplication( string id = CreateUUId(), array patterns = _defaultPatterns() ) {
		this.name = arguments.id;
		this.sessionManagement = false;
		variables.patterns = arguments.patterns;
	}
	public boolean function onRequestStart( required string targetPage ) {
		request.bootParentPing = _ping();
		request.bootParentVars = structKeyList( variables );
		return true;
	}
	private array function _defaultPatterns() { return [ "^/api/" ]; }
	private string function _ping() { return "pong"; }
}
