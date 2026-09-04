<cfscript>
suiteBegin("java.util.concurrent bounded pool");

// ============================================================
// ThreadPoolExecutor is not just somewhere to fling threads: the JVM runs at
// most maxPoolSize tasks at once, queues the overflow up to the work queue's
// capacity, and hands anything beyond that to a RejectedExecutionHandler.
//
// These properties are what Preside's cfconcurrent module is written against
// (system/externals/cfconcurrent), and none of them are observable from the
// module's own test suite — it never fills a queue. Running each submitted task
// immediately on its own detached thread passes every cfconcurrent spec while
// getting all of the back-pressure wrong, so it is asserted here instead.
// ============================================================

tmp       = getTempDirectory();
timeUnit  = createObject( "java", "java.util.concurrent.TimeUnit" );

function poolLog( required string name ) {
	var f = getTempDirectory() & "/rustcfml_pool_" & arguments.name & "_" & createUUID() & ".log";
	if ( fileExists( f ) ) { fileDelete( f ); }
	return f;
}

function makePool( required string policy, required numeric queueSize, numeric maxConcurrent=1 ) {
	var q   = createObject( "java", "java.util.concurrent.LinkedBlockingQueue" ).init( arguments.queueSize );
	var pol = createObject( "java", "java.util.concurrent.ThreadPoolExecutor$" & arguments.policy ).init();
	return createObject( "java", "java.util.concurrent.ThreadPoolExecutor" ).init(
		  arguments.maxConcurrent
		, arguments.maxConcurrent
		, 0
		, createObject( "java", "java.util.concurrent.TimeUnit" ).SECONDS
		, q
		, javacast( "null", "" )
		, pol
	);
}

function task( required string logFile, numeric ms=120 ) {
	return createDynamicProxy(
		  new java_shims.ConcurrentPoolTask( arguments.logFile, arguments.ms )
		, [ "java.util.concurrent.Callable" ]
	);
}

// Walk the start/end markers; the deepest nesting is the peak concurrency.
function peakConcurrency( required string logFile ) {
	var depth = 0;
	var peak  = 0;
	if ( !fileExists( arguments.logFile ) ) { return 0; }
	for ( var e in listToArray( fileRead( arguments.logFile ), chr(10) ) ) {
		if ( trim( e ) == "S" ) { depth++; if ( depth > peak ) { peak = depth; } }
		else if ( trim( e ) == "E" ) { depth--; }
	}
	return peak;
}

function startCount( required string logFile ) {
	var n = 0;
	if ( !fileExists( arguments.logFile ) ) { return 0; }
	for ( var e in listToArray( fileRead( arguments.logFile ), chr(10) ) ) {
		if ( trim( e ) == "S" ) { n++; }
	}
	return n;
}

// ---- maxConcurrent actually bounds concurrent execution ----
// Eight tasks into a 2-wide pool with a queue big enough to hold them all:
// every task must run, but never more than two at a time.
logA    = poolLog( "bound" );
poolA   = makePool( "DiscardPolicy", 100, 2 );
futures = [];
for ( i = 1; i <= 8; i++ ) { arrayAppend( futures, poolA.submit( task( logA, 120 ) ) ); }
for ( f in futures ) { f.get(); }
assert( "all 8 tasks ran", startCount( logA ), 8 );
assert( "peak concurrency is capped at maxConcurrent", peakConcurrency( logA ), 2 );
fileDelete( logA );

// ---- AbortPolicy throws once the work queue is full ----
logB     = poolLog( "abort" );
poolB    = makePool( "AbortPolicy", 2 );
accepted = 0;
caught   = "";
try {
	for ( i = 1; i <= 12; i++ ) { poolB.submit( task( logB, 400 ) ); accepted++; }
} catch ( any e ) {
	caught = e.type;
}
assert( "AbortPolicy raises RejectedExecutionException", caught, "java.util.concurrent.RejectedExecutionException" );
assertTrue( "AbortPolicy accepts only what fits (running + queued)", accepted > 0 && accepted < 12 );

// ---- DiscardPolicy drops the overflow instead of queueing it ----
logC  = poolLog( "discard" );
poolC = makePool( "DiscardPolicy", 2 );
fsC   = [];
for ( i = 1; i <= 12; i++ ) { arrayAppend( fsC, poolC.submit( task( logC, 100 ) ) ); }
for ( f in fsC ) { f.get(); }
assertTrue( "DiscardPolicy runs fewer tasks than were submitted", startCount( logC ) < 12 );
assertTrue( "a discarded task's future reports cancelled", arrayLen( fsC.filter( function( f ){ return f.isCancelled(); } ) ) > 0 );

// ---- CallerRunsPolicy drops nothing: the overflow runs on this thread ----
logD  = poolLog( "callerruns" );
poolD = makePool( "CallerRunsPolicy", 1 );
fsD   = [];
for ( i = 1; i <= 6; i++ ) { arrayAppend( fsD, poolD.submit( task( logD, 60 ) ) ); }
for ( f in fsD ) { f.get(); }
assert( "CallerRunsPolicy runs every submitted task", startCount( logD ), 6 );
fileDelete( logD );

// ---- a file created inside a task is visible to fileExists() afterwards ----
// `fileWrite` invalidates the existence cache BY PATH, and request_exists_cache
// is per-VM — so a write inside a thread's child VM used to clear only the
// CHILD's map, leaving the parent holding the negative it had cached before
// submitting. The parent then reported the file as absent while fileRead() on
// the very same path returned its contents. Deterministic, 15/15.
poolE = makePool( "DiscardPolicy", 100, 2 );
denied = 0;
unreadable = 0;
for ( i = 1; i <= 5; i++ ) {
	probePath = getTempDirectory() & "/rustcfml_pool_seen_" & createUUID() & ".txt";
	fileExists( probePath );  // caches the NEGATIVE that the write must retire
	poolE.submit(
		createDynamicProxy( new java_shims.ThreadFileWriter( probePath ), [ "java.util.concurrent.Callable" ] )
	).get();
	if ( !fileExists( probePath ) ) { denied++; }
	try { if ( fileRead( probePath ) != "hello" ) { unreadable++; } } catch ( any e ) { unreadable++; }
	fileDelete( probePath );
}
assert( "a file written inside a task is readable afterwards", unreadable, 0 );
assert( "fileExists() sees a file created inside a task", denied, 0 );

// ---- a periodic schedule's Future stays PENDING until cancelled ----
// The JVM's ScheduledFuture never completes on its own: `isDone()` is how a
// caller asks "is this schedule still live?". Preside's AbstractHeartBeat.start()
// is guarded by exactly `IsNull(future) || future.isDone() || future.isCancelled()`,
// so a Future that resolved after its FIRST tick made every heartbeat look
// stopped — and each start() scheduled another one on top. The observed result
// was the adhoc-task heartbeat running its DB-migration check hundreds of times
// per second.
tickLog = getTempDirectory() & "/rustcfml_pool_tick_" & createUUID() & ".log";
sched   = createObject( "java", "java.util.concurrent.ScheduledThreadPoolExecutor" ).init( 5, javacast( "null", "" ), javacast( "null", "" ) );
tickFuture = sched.scheduleAtFixedRate(
	  createDynamicProxy( new java_shims.ConcurrentTickTask( tickLog ), [ "java.lang.Runnable" ] )
	, 0
	, 60
	, timeUnit.MILLISECONDS
);
sleep( 300 );
assertFalse( "a live periodic schedule is not done", tickFuture.isDone() );
assertFalse( "a live periodic schedule is not cancelled", tickFuture.isCancelled() );

tickFuture.cancel( true );
sleep( 250 );
assertTrue( "a cancelled schedule reports cancelled", tickFuture.isCancelled() );
assertTrue( "a cancelled schedule reports done", tickFuture.isDone() );
ticksAtCancel = listLen( fileRead( tickLog ), chr(10) );
sleep( 300 );
assert( "a cancelled schedule stops firing", listLen( fileRead( tickLog ), chr(10) ), ticksAtCancel );
fileDelete( tickLog );

// ---- the TimeUnit argument is honoured, not assumed to be milliseconds ----
// The unit is the LAST argument; reading past it defaulted everything to
// MILLISECONDS, which turned cfconcurrent's 30-SECOND completion-queue poll
// into a 30ms one.
secLog = getTempDirectory() & "/rustcfml_pool_sec_" & createUUID() & ".log";
secFuture = sched.scheduleAtFixedRate(
	  createDynamicProxy( new java_shims.ConcurrentTickTask( secLog ), [ "java.lang.Runnable" ] )
	, 0
	, 1
	, timeUnit.SECONDS
);
sleep( 1400 );
secFuture.cancel( true );
secTicks = listLen( fileRead( secLog ), chr(10) );
assertTrue( "a 1-SECOND period fires ~twice in 1.4s, not hundreds of times (got " & secTicks & ")", secTicks >= 1 && secTicks <= 4 );
fileDelete( secLog );

suiteEnd();
</cfscript>
