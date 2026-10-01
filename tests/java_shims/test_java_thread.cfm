<cfscript>
suiteBegin("java.lang.Thread: interrupt flag, state, context class loader");

// Preside's ThreadUtil.isInterrupted() calls
// Thread.currentThread().isInterrupted() from every heartbeat; when the shim
// lacked it, each heartbeat threw at that check and did no work.

jThread = createObject( "java", "java.lang.Thread" );
cur     = jThread.currentThread();

assertFalse( "isInterrupted() is false by default", cur.isInterrupted() );

cur.interrupt();
r_after   = cur.isInterrupted();
r_again   = cur.isInterrupted();
r_static  = jThread.interrupted();
r_static2 = jThread.interrupted();
r_cleared = cur.isInterrupted();
assertTrue( "interrupt() sets the flag", r_after );
assertTrue( "isInterrupted() does not clear it", r_again );
assertTrue( "static interrupted() reports the flag", r_static );
assertFalse( "static interrupted() clears it", r_static2 );
assertFalse( "isInterrupted() after interrupted() is false", r_cleared );

assert( "getState().name() of the current thread", cur.getState().name(), "RUNNABLE" );
assert( "getState().toString()", cur.getState().toString(), "RUNNABLE" );
assert( "getState() in a string context", "" & cur.getState(), "RUNNABLE" );

cl = cur.getContextClassLoader();
assertFalse( "getContextClassLoader() returns an object", isNull( cl ) );
cur.setContextClassLoader( cl );
assertFalse( "setContextClassLoader() accepts it back", isNull( cur.getContextClassLoader() ) );

// Thread.sleep() really sleeps; interrupted, it throws and clears the flag.
// Nanosecond ticks are monotonic; millisecond ones follow the wall clock.
t0 = getTickCount( "nano" );
jThread.sleep( 150 );
sleptFor = int( ( getTickCount( "nano" ) - t0 ) / 1000000 );
assertTrue( "Thread.sleep() waits (slept " & sleptFor & "ms)", sleptFor >= 140 );
cur.interrupt();
sleepErr = "";
try {
	jThread.sleep( 2000 );
} catch ( any e ) {
	sleepErr = e.type & "|" & e.message;
}
assert( "an interrupted sleep throws InterruptedException", sleepErr, "java.lang.InterruptedException|sleep interrupted" );
assertFalse( "the interrupted sleep cleared the flag", cur.isInterrupted() );

// getStackTrace(): Java puts its own frame on top; the CFML frames follow.
function stackOfInner() {
	return createObject( "java", "java.lang.Thread" ).currentThread().getStackTrace();
}
st = stackOfInner();
assertTrue( "getStackTrace() returns frames", arrayLen( st ) > 1 );
assert( "the top frame is getStackTrace", st[ 1 ].getMethodName(), "getStackTrace" );
inThisFile = 0;
for ( el in st ) {
	if ( ( el.getFileName() ?: "" ) contains "test_java_thread.cfm" && el.getLineNumber() > 0 ) { inThisFile++; }
}
assertTrue( "the CFML frames name this template and a line", inThisFile >= 1 );

// A method the shim does not have throws, naming the Java method.
missErr = "";
try {
	cur.noSuchThreadMethod();
} catch ( any e ) {
	missErr = e.message;
}
assertTrue( "an unsupported Thread method throws naming it", missErr contains "noSuchThreadMethod" );

// Each task starts with a clear flag, and a task's interrupt never reaches the
// request that submitted it (ThreadPoolExecutor clears it between tasks).
pool  = createObject( "java", "java.util.concurrent.Executors" ).newFixedThreadPool( 1 );
first = pool.submit( createDynamicProxy( new java_shims.ThreadInterruptTask( true ), [ "java.util.concurrent.Callable" ] ) );
r_first = first.get();
second  = pool.submit( createDynamicProxy( new java_shims.ThreadInterruptTask( false ), [ "java.util.concurrent.Callable" ] ) );
r_second = second.get();
pool.shutdown();
assertTrue( "a task sees its own interrupt", r_first );
assertFalse( "the next task on the same pool starts uninterrupted", r_second );
assertFalse( "the submitting request is not interrupted", jThread.currentThread().isInterrupted() );

suiteEnd();
</cfscript>
