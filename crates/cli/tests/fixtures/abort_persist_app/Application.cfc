component {
	this.name = "abort_persist_app";

	public boolean function onApplicationStart() {
		application.holder = {};
		return true;
	}

	public boolean function onRequestStart( required string targetPage ) {
		if ( StructKeyExists( url, "cacheAndAbort" ) ) {
			application.holder.nested = new Child();
			application.topLevel      = new Child();
			writeOutput( "primed:" & application.holder.nested.greet() );
			abort;
		}
		return true;
	}
}
