/**
 * Fixture for the cross-VM existence-cache regression in
 * test_java_util_concurrent_pool.cfm: writes one file from inside whatever
 * thread the executor runs it on.
 */
component {

	function init( required string path ) {
		variables.path = arguments.path;
		return this;
	}

	function call() {
		fileWrite( variables.path, "hello" );
		return true;
	}

}
