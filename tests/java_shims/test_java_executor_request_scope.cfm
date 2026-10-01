<cfscript>
suiteBegin("java.util.concurrent tasks get their own request scope");

// On Lucee each executor task run has its own page context: `request` starts
// empty and is discarded when the run ends. Sharing the submitting request's
// scope let concurrent runs overwrite each other and left their values behind,
// which stopped Preside's heartbeats (they keep the ColdBox request context in
// `request`). cfthread is different: it shares the parent's request scope.

function rsTask( required string id, string logFile="" ) {
	return createDynamicProxy( new java_shims.RequestScopeTask( arguments.id, arguments.logFile ), [ "java.util.concurrent.Callable" ] );
}

request.__rsTask = "creator";
try { session.__rsMark = true; } catch ( any e ) {}

// submit(): 12 tasks across 4 threads.
pool = createObject( "java", "java.util.concurrent.Executors" ).newFixedThreadPool( 4 );
futs = [];
for ( i = 1; i <= 12; i++ ) {
	arrayAppend( futs, pool.submit( rsTask( "t" & i ) ) );
}
foundOther  = 0;
overwritten = 0;
sawSession  = 0;
for ( f in futs ) {
	r = f.get();
	if ( len( r.found ) ) { foundOther++; }
	if ( !r.kept ) { overwritten++; }
	if ( r.sawSession ) { sawSession++; }
}
pool.shutdown();
assert( "no task sees another task's or the submitter's request values", foundOther, 0 );
assert( "no task has its request values overwritten mid-run", overwritten, 0 );
assert( "the submitting request's scope is untouched", request.__rsTask, "creator" );
assert( "no task sees the submitter's session", sawSession, 0 );

// scheduleAtFixedRate(): each tick starts clean.
logFile = getTempDirectory() & "/rustcfml_rs_" & createUUID() & ".log";
ses = createObject( "java", "java.util.concurrent.Executors" ).newScheduledThreadPool( 1 );
tu  = createObject( "java", "java.util.concurrent.TimeUnit" );
sf  = ses.scheduleAtFixedRate(
	  createDynamicProxy( new java_shims.RequestScopeTask( "tick", logFile ), [ "java.lang.Runnable" ] )
	, 0, 60, tu.MILLISECONDS
);
sleep( 350 );
sf.cancel( true );
ses.shutdown();
sleep( 100 );
ticks = fileExists( logFile ) ? listToArray( fileRead( logFile ), chr( 10 ) ) : [];
if ( fileExists( logFile ) ) { fileDelete( logFile ); }
assertTrue( "the periodic task ran more than once", arrayLen( ticks ) >= 2 );
assert( "no tick sees the previous tick's request values", arrayLen( arrayFilter( ticks, function( t ) { return listFirst( t ) != "clean"; } ) ), 0 );
assert( "the schedule never writes the submitter's request scope", request.__rsTask, "creator" );

// Preside reaches the executor through cfconcurrent's LuceeRunnable, which
// RustCFML shims (Lucee needs Preside's jar for it).
if ( isRustCFML() ) {
	lr   = createObject( "java", "org.pixl8.cfconcurrent.LuceeRunnable" ).init( new java_shims.RequestScopeTask( "lr" ), expandPath( "/" ), "", "" );
	pool = createObject( "java", "java.util.concurrent.Executors" ).newFixedThreadPool( 1 );
	pool.submit( lr ).get();
	pool.shutdown();
	assert( "a LuceeRunnable task does not write the submitter's request scope", request.__rsTask, "creator" );
}

// cfthread keeps sharing the parent's request scope.
thread name="rsThread" {
	request.__rsThread = "fromThread";
}
thread action="join" name="rsThread";
assert( "cfthread still shares the request scope", request.__rsThread ?: "", "fromThread" );

suiteEnd();
</cfscript>
