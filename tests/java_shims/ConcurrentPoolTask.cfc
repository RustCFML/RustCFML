/**
 * Fixture for test_java_util_concurrent_pool.cfm. Records a start and an end
 * marker around a sleep, so the test can reconstruct peak concurrency by
 * walking the markers in order.
 */
component {

	function init( required string logFile, numeric sleepMs=120 ) {
		variables.logFile = arguments.logFile;
		variables.sleepMs = arguments.sleepMs;
		return this;
	}

	function call() {
		fileAppend( variables.logFile, "S" & chr(10) );
		sleep( variables.sleepMs );
		fileAppend( variables.logFile, "E" & chr(10) );
		return true;
	}

	function run() {
		call();
	}

}
