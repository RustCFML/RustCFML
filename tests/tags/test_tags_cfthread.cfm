<cfscript>
suiteBegin("cfthread Tag");
</cfscript>

<!--- cfthread action=run (default) --->
<cfthread name="t1">
    <cfset request.threadTest1 = "hello from thread">
</cfthread>
<cfthread action="join" name="t1" timeout="5000"/>

<cfscript>
assert("cfthread run sets variable", request.threadTest1, "hello from thread");
</cfscript>

<!--- cfthread with explicit action=run --->
<cfthread name="t2" action="run">
    <cfset request.threadTest2 = "explicit run">
</cfthread>
<cfthread action="join" name="t2" timeout="5000"/>

<cfscript>
assert("cfthread explicit action=run", request.threadTest2, "explicit run");
</cfscript>

<cfscript>
assert("cfthread join completes", true, true);
</cfscript>

<!--- cfthread terminate (no-op since already complete) --->
<cfthread action="terminate" name="t1"/>

<cfscript>
assert("cfthread terminate completes", true, true);

// Check thread scope metadata
assertTrue("cfthread scope exists", isDefined("cfthread"));
assertTrue("cfthread.t1 exists", isDefined("cfthread.t1"));
assert("cfthread.t1 status", cfthread.t1.status, "COMPLETED");
</cfscript>

<!--- Output capture --->
<cfthread name="t3">
    <cfoutput>hello from thread</cfoutput>
</cfthread>
<cfthread action="join" name="t3" timeout="5000"/>

<cfscript>
assert("cfthread output capture", trim(cfthread.t3.output), "hello from thread");
</cfscript>

<!--- Error capture --->
<cfthread name="t4">
    <cfthrow message="thread error test">
</cfthread>
<cfthread action="join" name="t4" timeout="5000"/>

<cfscript>
assertTrue("cfthread error captured", len(cfthread.t4.error) > 0);
assert("cfthread status after error", cfthread.t4.status, "TERMINATED");
</cfscript>

<!--- Elapsed time --->
<cfscript>
assertTrue("cfthread elapsedtime is numeric", isNumeric(cfthread.t3.elapsedtime));
</cfscript>

<!--- Thread scope --->
<cfthread name="t5">
    <cfset thread.result = "done">
    <cfset thread.count = 42>
</cfthread>
<cfthread action="join" name="t5" timeout="5000"/>

<cfscript>
assert("cfthread thread scope string", cfthread.t5.result, "done");
assert("cfthread thread scope number", cfthread.t5.count, 42);
</cfscript>

<!--- Pre-seeded thread scope visible inside the thread (issue #217) --->
<cfset thread.seeded = "HELLO-FROM-OUTSIDE">
<cfthread name="t6">
    <cfset thread.echo = isNull(thread.seeded) ? "NULL-INSIDE-THREAD" : thread.seeded>
</cfthread>
<cfthread action="join" name="t6" timeout="5000"/>

<cfscript>
// RustCFML-only: Lucee 7.1 does NOT carry a pre-seeded thread.* value into the
// thread (it reads as null there, at page level and inside a function alike).
if ( isRustCFML() ) {
	assert("pre-seeded thread scope visible inside thread", cfthread.t6.echo, "HELLO-FROM-OUTSIDE");
}

suiteEnd();
</cfscript>
