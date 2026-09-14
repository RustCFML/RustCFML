component {
	variables.someState = 1;

	// level 0 = plain construction
	//       1 = + getMetaData(this)
	//       2 = + the whole-variables-scope copy into `core`
	//       3 = + the three mixin StructAppends   (the full Wheels body)
	public any function init( numeric level = 0 ) {
		if ( arguments.level GT 0 ) { $initializeMixins( variables, arguments.level ); }
		return this;
	}

	public any function $initializeMixins( required struct variablesScope, required numeric level ) {
		var $wheels = {};
		$wheels.metaData = getMetaData( variablesScope.this );
		$wheels.className = "model";
		if ( arguments.level LT 2 ) { return variablesScope; }
		if ( structKeyExists( application.mixins, $wheels.className ) ) {
			if ( !structKeyExists( variablesScope, "core" ) ) {
				variablesScope.core = {};
				structAppend( variablesScope.core, variablesScope );
				structDelete( variablesScope.core, "$wheels" );
			}
			if ( arguments.level LT 3 ) { return variablesScope; }
			structAppend( variablesScope, application.mixins[ $wheels.className ], true );
			if ( structKeyExists( variablesScope, "this" ) ) {
				structAppend( variablesScope.this, application.mixins[ $wheels.className ], true );
			}
			if ( structKeyExists( variablesScope.core, "this" ) ) {
				structAppend( variablesScope.core.this, application.mixins[ $wheels.className ], true );
			}
		}
		return variablesScope;
	}

	public string function m001( a, b ) { return "m001" & a & b; }
	public string function m002( a, b ) { return "m002" & a & b; }
	public string function m003( a, b ) { return "m003" & a & b; }
	public string function m004( a, b ) { return "m004" & a & b; }
	public string function m005( a, b ) { return "m005" & a & b; }
	public string function m006( a, b ) { return "m006" & a & b; }
	public string function m007( a, b ) { return "m007" & a & b; }
	public string function m008( a, b ) { return "m008" & a & b; }
	public string function m009( a, b ) { return "m009" & a & b; }
	public string function m010( a, b ) { return "m010" & a & b; }
	public string function m011( a, b ) { return "m011" & a & b; }
	public string function m012( a, b ) { return "m012" & a & b; }
	public string function m013( a, b ) { return "m013" & a & b; }
	public string function m014( a, b ) { return "m014" & a & b; }
	public string function m015( a, b ) { return "m015" & a & b; }
	public string function m016( a, b ) { return "m016" & a & b; }
	public string function m017( a, b ) { return "m017" & a & b; }
	public string function m018( a, b ) { return "m018" & a & b; }
	public string function m019( a, b ) { return "m019" & a & b; }
	public string function m020( a, b ) { return "m020" & a & b; }
}
