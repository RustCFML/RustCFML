<cfscript>
suiteBegin("java.util.concurrent shim + createDynamicProxy (ColdBox/Preside pattern)");

// This exercises the exact pattern ColdBox's async layer uses: build an executor
// from Executors, wrap a Callable CFC as a Java SAM via createDynamicProxy, submit
// it, and read the result off the returned Future. It runs on BOTH Lucee (real
// JVM executor + proxy) and RustCFML (java.util.concurrent shim routed through the
// native async kernel) — a cross-engine compatibility check.

executors = createObject( "java", "java.util.concurrent.Executors" );
pool      = executors.newFixedThreadPool( 2 );

callable = new concurrenttest.SampleCallable();
proxy    = createDynamicProxy( callable, [ "java.util.concurrent.Callable" ] );

future = pool.submit( proxy );
assert( "Callable proxy submitted to executor, future.get() returns its value", future.get(), "called!" );
assertTrue( "future.isDone() is true after get()", future.isDone() );

// TimeUnit constants: on Lucee these are real enum values, on RustCFML opaque
// string tokens; both stringify to the constant name.
timeUnit = createObject( "java", "java.util.concurrent.TimeUnit" );
assert( "TimeUnit.SECONDS.toString()", timeUnit.SECONDS.toString(), "SECONDS" );

// Lifecycle: shutting down an executor is accepted on both engines.
pool.shutdown();

suiteEnd();
</cfscript>
