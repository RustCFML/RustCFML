component {
	function init() { variables.router = new Router(); return this; }
	// A captured CFC used as the ROOT of a mutating member-call chain inside an
	// each() callback: `.append()` on the call result must never write the
	// array back over the captured component (Preside ModuleService shape).
	function activate( required string moduleName ) {
		var appRouter = variables.router;
		var routes = [ { pattern: "/a", module: "" }, { pattern: "/b", module: "x" } ];
		routes.each( function( item ){
			if ( !item.module.len() ) { item.module = moduleName; }
			appRouter.getModuleRoutes( moduleName ).append( item );
		} );
		return appRouter.getModuleRoutes( arguments.moduleName ).len() & "/" & isObject( appRouter );
	}
}
