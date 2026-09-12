component {
	function init() { variables.routes = []; return this; }
	public function getRoutes() { return variables.routes; }
	public function addRoute( required string pattern ) {
		arrayAppend( variables.routes, { pattern: arguments.pattern, constraints: {} } );
		return this;
	}
	// A `var` local assigned the very array the component scope holds, then
	// read back and mutated through the local — the Wheels
	// `$applyConstraintToLastRoute` shape.
	public function constrainLast( required string variableName, required string pattern ) {
		local.routes = this.getRoutes();
		local.count = arrayLen( local.routes );
		local.route = local.routes[ local.count ];
		local.route.constraints[ arguments.variableName ] = arguments.pattern;
		local.routes[ local.count ] = local.route;
		return local.count;
	}
	public function shareThenShadow() {
		var shared = variables.routes;
		var n = arrayLen( shared );
		return n;
	}
}
