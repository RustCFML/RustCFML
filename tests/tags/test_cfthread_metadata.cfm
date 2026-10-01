<cfscript>
suiteBegin("cfthread: the cfthread.NAME struct");

// Lucee's keys, while running and once done. `error` exists only when the
// thread failed, so `structKeyExists( t, "error" )` tells failure from success.

thread name="mdRunning" {
	sleep( 300 );
}
running = cfthread.mdRunning;
assert( "a running thread's status", running.status, "RUNNING" );
// (Not `childThreads`: Lucee adds it to a running thread a little late.)
for ( k in [ "elapsedTime", "interrupted", "name", "output", "priority", "stackTrace", "startTime", "status" ] ) {
	assertTrue( "a running thread has [" & k & "]", structKeyExists( running, k ) );
}
assertFalse( "a running thread has no error", structKeyExists( running, "error" ) );
thread action="join" name="mdRunning";

thread name="mdOk" {
	thread.answer = 42;
}
thread action="join" name="mdOk";
ok = cfthread.mdOk;
assert( "status after success", ok.status, "COMPLETED" );
assertFalse( "no error key after success", structKeyExists( ok, "error" ) );
assert( "default priority", ok.priority, "NORMAL" );
assertTrue( "startTime is a date", isDate( ok.startTime ) );
assertFalse( "interrupted is false", ok.interrupted );
assertTrue( "childThreads is a struct", isStruct( ok.childThreads ) );
assert( "thread.* values are kept", ok.answer, 42 );

thread name="mdBad" {
	throw( message="mdBoom", type="MdType", detail="mdDetail" );
}
thread action="join" name="mdBad";
bad = cfthread.mdBad;
assert( "status after a throw", bad.status, "TERMINATED" );
assertTrue( "error key after a throw", structKeyExists( bad, "error" ) );
assertTrue( "error is a struct", isStruct( bad.error ) );
assert( "error.message", bad.error.message, "mdBoom" );
assert( "error.type", bad.error.type, "MdType" );
assert( "error.detail", bad.error.detail, "mdDetail" );

thread name="mdHigh" priority="HIGH" {
	x = 1;
}
thread action="join" name="mdHigh";
assert( "the priority attribute", cfthread.mdHigh.priority, "HIGH" );

thread name="mdLowAttrs" priority="low" foo="bar" {
	thread.got = attributes.foo;
}
thread action="join" name="mdLowAttrs";
assert( "priority is upper-cased", cfthread.mdLowAttrs.priority, "LOW" );
assert( "custom attributes still reach the body", cfthread.mdLowAttrs.got, "bar" );

</cfscript>
<cfthread name="mdTagHigh" priority="HIGH" foo="tagBar">
	<cfset thread.got = attributes.foo>
</cfthread>
<cfthread action="join" name="mdTagHigh" />
<cfscript>
assert( "the tag form passes priority", cfthread.mdTagHigh.priority, "HIGH" );
assert( "the tag form still passes custom attributes", cfthread.mdTagHigh.got, "tagBar" );
suiteEnd();
</cfscript>
