/**
 * Fixture for test_java_executor_request_scope.cfm. Each run records whether it
 * found another run's (or the submitting request's) marker in `request`, sets
 * its own, pauses, and checks nobody overwrote it. With a log file it appends
 * one line per run, which is how a periodic schedule reports back.
 */
component {

	function init( required string id, string logFile="" ) {
		variables.id      = arguments.id;
		variables.logFile = arguments.logFile;
		return this;
	}

	function call() {
		var found = request.__rsTask ?: "";
		request.__rsTask = variables.id;
		sleep( 20 );
		var kept = ( request.__rsTask ?: "" ) == variables.id;
		if ( len( variables.logFile ) ) {
			fileAppend( variables.logFile, ( len( found ) ? "found" : "clean" ) & "," & ( kept ? "kept" : "lost" ) & chr( 10 ) );
		}
		var sawSession = false;
		try { sawSession = structKeyExists( session, "__rsMark" ); } catch ( any e ) {}
		return { found = found, kept = kept, sawSession = sawSession };
	}

	function run() {
		call();
	}

}
