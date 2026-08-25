/**
 * Stands in for a framework bootstrap CFC that an application's Application.cfc
 * extends and configures with a single `super.setupApplication(...)` call —
 * the shape Preside, FW/1 and ColdBox all use.
 *
 * The point of the fixture is the DEFAULT ARGUMENTS: each one calls a sibling
 * method on this same component by bare name, so evaluating them requires the
 * parent method's frame to have a variables scope carrying the method table.
 */
component {

	public void function setupApplication(
		  string  id                       = "unnamed"
		, array   statelessUrlPatterns     = _getDefaultStatelessUrlPatterns()
		, boolean presideSessionManagement = _useSessionManagement()
		, string  viaVariablesScope        = variables._getDefaultStatelessUrlPatterns()[ 1 ]
	) {
		this.name                = arguments.id;
		this.sessionManagement   = arguments.presideSessionManagement;
		request._patterns        = arguments.statelessUrlPatterns;
		request._viaVariables    = arguments.viaVariablesScope;
		// A bare sibling call from the BODY of the parent method, not just from a
		// default-argument expression.
		request._fromBody        = _useSessionManagement();
	}

	private array function _getDefaultStatelessUrlPatterns() {
		return [ "^/api/", "^/asset/" ];
	}

	private boolean function _useSessionManagement() {
		return true;
	}
}
