component {
	this.name = "cold_app_create_app";

	// Preside's shape: no onApplicationStart; the first request boots under an
	// exclusive lock and re-checks inside it, so it must boot exactly once.
	public boolean function onRequestStart( required string targetPage ) {
		if ( !StructKeyExists( application, "booted" ) ) {
			lock name="cold_app_create_boot" type="exclusive" timeout="0" {
				if ( !StructKeyExists( application, "booted" ) ) {
					server.coldAppCreateBoots = ( server.coldAppCreateBoots ?: 0 ) + 1;
					sleep( 300 );
					application.booted = true;
				}
			}
		}
		return true;
	}
}
