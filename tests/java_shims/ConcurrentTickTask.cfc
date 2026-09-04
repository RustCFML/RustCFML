/**
 * Fixture for the periodic-scheduling assertions in
 * test_java_util_concurrent_pool.cfm. One line per run, so the caller can count
 * how many times a schedule actually fired.
 */
component {

	function init( required string logFile ) {
		variables.logFile = arguments.logFile;
		return this;
	}

	function run() {
		fileAppend( variables.logFile, "tick" & chr(10) );
	}

}
