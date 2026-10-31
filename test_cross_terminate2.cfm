<cfscript>
// Cross-thread terminate test with error checking
request.joinerDone = false;
request.targetReached = false;
request.joinerError = "";

thread name="target" {
    sleep(10000);
    thread.done = true;
    request.targetReached = true;
}

sleep(100);

thread name="joiner" {
    try {
        thread action="terminate" name="target";
        sleep(200);
        thread action="join" name="target" timeout="5000";
        request.joinerDone = true;
    } catch (any e) {
        request.joinerError = e.message;
    }
}

thread action="join" name="joiner,target" timeout="15000";

writeDump("Joiner done: " & request.joinerDone);
writeDump("Joiner error: " & request.joinerError);
writeDump("Target status: " & cfthread.target.status);
writeDump("Target elapsed: " & cfthread.target.elapsedTime);
writeDump("Target reached: " & request.targetReached);

if (structKeyExists(cfthread, "joiner") && structKeyExists(cfthread.joiner, "error")) {
    writeDump("Joiner cfthread error: " & cfthread.joiner.error);
}
</cfscript>