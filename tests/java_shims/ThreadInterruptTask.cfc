/**
 * Fixture for test_java_thread.cfm. Optionally interrupts the thread it runs
 * on, then reports that thread's interrupt flag.
 */
component {

	function init( boolean interruptSelf=false ) {
		variables.interruptSelf = arguments.interruptSelf;
		return this;
	}

	function call() {
		var t = createObject( "java", "java.lang.Thread" ).currentThread();
		if ( variables.interruptSelf ) {
			t.interrupt();
		}
		return t.isInterrupted();
	}

}
