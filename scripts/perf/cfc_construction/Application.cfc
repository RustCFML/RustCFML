component {
	this.name = "rustcfml_cfc_construction_bench";
	function onApplicationStart() {
		var mp = createObject( "component", "scripts.perf.cfc_construction.MixinProvider" );
		var m = {};
		for ( var f in getMetaData( mp ).functions ) { m[ f.name ] = mp[ f.name ]; }
		application.mixins = { model = m };
		return true;
	}
	function onRequestStart() {
		if ( !structKeyExists( application, "mixins" ) ) { onApplicationStart(); }
		return true;
	}
}
