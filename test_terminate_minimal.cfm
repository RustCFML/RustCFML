<cfscript>
// Minimal terminate test
request.done = false;
request.reached = false;

thread name="term_test" {
    sleep(5000);
    thread.done = true;
    request.reached = true;
}

sleep(100);

writeDump("Before terminate");
thread action="terminate" name="term_test";
writeDump("After terminate");

sleep(200);

writeDump("After wait");
thread action="join" name="term_test" timeout="10000";

writeDump("Status: " & cfthread.term_test.status);
writeDump("Elapsed: " & cfthread.term_test.elapsedTime);
writeDump("Reached: " & request.reached);
</cfscript>