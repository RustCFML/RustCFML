<cfscript>
// Cross-thread terminate test
request.joinerDone = false;
request.targetReached = false;

thread name="target" {
    sleep(10000);
    thread.done = true;
    request.targetReached = true;
}

sleep(100);

thread name="joiner" {
    writeDump("Joiner: before terminate");
    thread action="terminate" name="target";
    writeDump("Joiner: after terminate");
    sleep(200);
    thread action="join" name="target" timeout="5000";
    writeDump("Joiner: after join");
    request.joinerDone = true;
}

thread action="join" name="joiner,target" timeout="15000";

writeDump("Joiner done: " & request.joinerDone);
writeDump("Target status: " & cfthread.target.status);
writeDump("Target elapsed: " & cfthread.target.elapsedTime);
writeDump("Target reached: " & request.targetReached);
</cfscript>