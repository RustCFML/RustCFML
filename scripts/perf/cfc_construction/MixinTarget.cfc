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
	public string function m021( a, b ) { return "m021" & a & b; }
	public string function m022( a, b ) { return "m022" & a & b; }
	public string function m023( a, b ) { return "m023" & a & b; }
	public string function m024( a, b ) { return "m024" & a & b; }
	public string function m025( a, b ) { return "m025" & a & b; }
	public string function m026( a, b ) { return "m026" & a & b; }
	public string function m027( a, b ) { return "m027" & a & b; }
	public string function m028( a, b ) { return "m028" & a & b; }
	public string function m029( a, b ) { return "m029" & a & b; }
	public string function m030( a, b ) { return "m030" & a & b; }
	public string function m031( a, b ) { return "m031" & a & b; }
	public string function m032( a, b ) { return "m032" & a & b; }
	public string function m033( a, b ) { return "m033" & a & b; }
	public string function m034( a, b ) { return "m034" & a & b; }
	public string function m035( a, b ) { return "m035" & a & b; }
	public string function m036( a, b ) { return "m036" & a & b; }
	public string function m037( a, b ) { return "m037" & a & b; }
	public string function m038( a, b ) { return "m038" & a & b; }
	public string function m039( a, b ) { return "m039" & a & b; }
	public string function m040( a, b ) { return "m040" & a & b; }
	public string function m041( a, b ) { return "m041" & a & b; }
	public string function m042( a, b ) { return "m042" & a & b; }
	public string function m043( a, b ) { return "m043" & a & b; }
	public string function m044( a, b ) { return "m044" & a & b; }
	public string function m045( a, b ) { return "m045" & a & b; }
	public string function m046( a, b ) { return "m046" & a & b; }
	public string function m047( a, b ) { return "m047" & a & b; }
	public string function m048( a, b ) { return "m048" & a & b; }
	public string function m049( a, b ) { return "m049" & a & b; }
	public string function m050( a, b ) { return "m050" & a & b; }
	public string function m051( a, b ) { return "m051" & a & b; }
	public string function m052( a, b ) { return "m052" & a & b; }
	public string function m053( a, b ) { return "m053" & a & b; }
	public string function m054( a, b ) { return "m054" & a & b; }
	public string function m055( a, b ) { return "m055" & a & b; }
	public string function m056( a, b ) { return "m056" & a & b; }
	public string function m057( a, b ) { return "m057" & a & b; }
	public string function m058( a, b ) { return "m058" & a & b; }
	public string function m059( a, b ) { return "m059" & a & b; }
	public string function m060( a, b ) { return "m060" & a & b; }
	public string function m061( a, b ) { return "m061" & a & b; }
	public string function m062( a, b ) { return "m062" & a & b; }
	public string function m063( a, b ) { return "m063" & a & b; }
	public string function m064( a, b ) { return "m064" & a & b; }
	public string function m065( a, b ) { return "m065" & a & b; }
	public string function m066( a, b ) { return "m066" & a & b; }
	public string function m067( a, b ) { return "m067" & a & b; }
	public string function m068( a, b ) { return "m068" & a & b; }
	public string function m069( a, b ) { return "m069" & a & b; }
	public string function m070( a, b ) { return "m070" & a & b; }
	public string function m071( a, b ) { return "m071" & a & b; }
	public string function m072( a, b ) { return "m072" & a & b; }
	public string function m073( a, b ) { return "m073" & a & b; }
	public string function m074( a, b ) { return "m074" & a & b; }
	public string function m075( a, b ) { return "m075" & a & b; }
	public string function m076( a, b ) { return "m076" & a & b; }
	public string function m077( a, b ) { return "m077" & a & b; }
	public string function m078( a, b ) { return "m078" & a & b; }
	public string function m079( a, b ) { return "m079" & a & b; }
	public string function m080( a, b ) { return "m080" & a & b; }
	public string function m081( a, b ) { return "m081" & a & b; }
	public string function m082( a, b ) { return "m082" & a & b; }
	public string function m083( a, b ) { return "m083" & a & b; }
	public string function m084( a, b ) { return "m084" & a & b; }
	public string function m085( a, b ) { return "m085" & a & b; }
	public string function m086( a, b ) { return "m086" & a & b; }
}
