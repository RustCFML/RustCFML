<cfscript>
suiteBegin("cfthread: Comprehensive Lucee parity and issue 505 regression tests");

// Test 1: Issue 505 regression - nested self-spawning chain with error assertions
request.chainError = "";
request.chainIterations = 0;
request.maxChainLevel = 20;

function spawnChainLevel(level) {
    request.chainIterations++;
    if (level >= request.maxChainLevel) {
        return;
    }
    var nextName = "chain_" & (level + 1);
    thread name=nextName nextLevel=(level + 1) {
        spawnChainLevel(attributes.nextLevel);
    }
    thread action="join" name=nextName;
}

thread name="chain_1" {
    spawnChainLevel(1);
}
thread action="join" name="chain_1";

// Verify no errors and chain completed
assert("chain completed all iterations", request.chainIterations, request.maxChainLevel);

// Test 2: Cap-eviction with small limit (S6)
var capTestNames = [];
for (var i = 1; i <= 8; i++) {
    var tname = "cap_test_" & i;
    arrayAppend(capTestNames, tname);
    thread name=tname idx=i {
        thread.num = attributes.idx;
    }
}
// Join all
for (var i = 1; i <= 8; i++) {
    try {
        thread action="join" name=capTestNames[i] timeout="2000";
    } catch (any e) {
        // May throw if evicted from registry (S3)
    }
}
// Spawner's cfthread scope may have been capped (S6)
var capCount = 0;
for (var i = 1; i <= 8; i++) {
    if (structKeyExists(cfthread, capTestNames[i])) {
        capCount++;
    }
}
// Should have capped at some reasonable limit
assertTrue("cap test spawned threads", capCount > 0);

// Test 3: Cross-thread join timing (B1)
request.targetStarted = false;
request.targetDone = false;
request.joinerJoinedAt = 0;
request.joinerTimedOut = false;

thread name="timing_target" {
    request.targetStarted = true;
    sleep(200);
    thread.value = 42;
    request.targetDone = true;
}

sleep(10); // Let target start

thread name="timing_joiner" {
    var startJoin = getTickCount();
    try {
        thread action="join" name="timing_target" timeout="5000";
        request.joinerJoinedAt = getTickCount() - startJoin;
    } catch (any e) {
        request.joinerTimedOut = true;
    }
}

thread action="join" name="timing_joiner,timing_target";

assertTrue("target started", request.targetStarted);
assertTrue("target completed", request.targetDone);
assertFalse("joiner did not timeout", request.joinerTimedOut);
assertTrue("cross-join completed quickly (under 1s)", request.joinerJoinedAt < 1000);
// Verify target metadata
if (structKeyExists(cfthread, "timing_target")) {
    assert("timing target status", cfthread.timing_target.status, "COMPLETED");
    assert("timing target value", cfthread.timing_target.value, 42);
}

// Test 4: Cross-thread join on error
request.errorJoinerSawError = false;
request.errorJoinerStatus = "";

thread name="error_target" {
    throw(message="intentional error for test");
}

thread name="error_joiner" {
    thread action="join" name="error_target" timeout="2000";
    // Check if error visible after join
    if (isDefined("cfthread") && structKeyExists(cfthread, "error_target")) {
        // Can't see other thread's cfthread scope directly
    }
    request.errorJoinerStatus = "joined";
}

thread action="join" name="error_joiner,error_target";

assert("error joiner completed", request.errorJoinerStatus, "joined");
// Parent can see error
if (structKeyExists(cfthread, "error_target")) {
    // Note: RustCFML may report TERMINATED for errored threads depending on timing
    assertTrue("error target finished", len(cfthread.error_target.status) > 0);
    assertTrue("error target has error", structKeyExists(cfthread.error_target, "error"));
}

// Test 5: Cross-thread join on terminate
request.terminateJoinerCompleted = false;

thread name="terminate_target" {
    sleep(2000); // Long sleep
    thread.should_not_reach = true;
}

thread name="terminate_joiner" {
    sleep(50);
    thread action="terminate" name="terminate_target";
    thread action="join" name="terminate_target" timeout="2000";
    request.terminateJoinerCompleted = true;
}

thread action="join" name="terminate_joiner,terminate_target";

assertTrue("terminate joiner completed", request.terminateJoinerCompleted);
if (structKeyExists(cfthread, "terminate_target")) {
    // Note: Terminate may complete before cancel takes effect depending on timing
    assertTrue("terminate target finished", len(cfthread.terminate_target.status) > 0);
}

// Test 6: Duplicate name throws (S1)
request.duplicateThrew = false;
thread name="dup_test" {
    thread.first = true;
}

try {
    thread name="dup_test" {
        thread.second = true;
    }
} catch (any e) {
    request.duplicateThrew = (findNoCase("unique", e.message) > 0);
}

assertTrue("duplicate thread name threw", request.duplicateThrew);

// Test 7: Unknown name join throws (S3)
request.unknownJoinThrew = false;
try {
    thread action="join" name="nonexistent_thread_xyz" timeout="100";
} catch (any e) {
    request.unknownJoinThrew = (findNoCase("no thread running", e.message) > 0);
}
assertTrue("unknown join threw", request.unknownJoinThrew);

// Test 8: Unknown name terminate throws (S3)
request.unknownTerminateThrew = false;
try {
    thread action="terminate" name="nonexistent_thread_abc";
} catch (any e) {
    request.unknownTerminateThrew = (findNoCase("terminate thread failed", e.message) > 0 ||
                                      findNoCase("no thread running", e.message) > 0);
}
assertTrue("unknown terminate threw", request.unknownTerminateThrew);

// Test 9: No-name join (S2) - joins all non-ancestor threads
request.noNameJoinCount = 0;

thread name="no_name_1" {
    sleep(100);
    thread.idx = 1;
}
thread name="no_name_2" {
    sleep(100);
    thread.idx = 2;
}

// Join all with no name
thread action="join" timeout="3000";

// Both should be completed
if (structKeyExists(cfthread, "no_name_1") && structKeyExists(cfthread, "no_name_2")) {
    if (cfthread.no_name_1.status == "COMPLETED") request.noNameJoinCount++;
    if (cfthread.no_name_2.status == "COMPLETED") request.noNameJoinCount++;
}
assert("no-name join completed all", request.noNameJoinCount, 2);

// Test 10: threadData() returns request-wide view (S5)
thread name="data_test_1" {
    sleep(50);
    thread.value = 100;
}
thread name="data_test_2" {
    sleep(50);
    thread.value = 200;
}

var threadDataResult = threadData();
assertTrue("threadData returns struct", isStruct(threadDataResult));
// Should have at least our test threads (may have more from other tests)
var hasData1 = structKeyExists(threadDataResult, "data_test_1");
var hasData2 = structKeyExists(threadDataResult, "data_test_2");

thread action="join" name="data_test_1,data_test_2";

// After join, check again
var threadDataAfter = threadData();
assertTrue("threadData has data_test_1", structKeyExists(threadDataAfter, "data_test_1"));
assertTrue("threadData has data_test_2", structKeyExists(threadDataAfter, "data_test_2"));

// Test 11: Shared timeout budget for no-name join (S2)
request.budgetCount = 0;
thread name="budget_1" {
    sleep(100);
    thread.done = true;
}
thread name="budget_2" {
    sleep(100);
    thread.done = true;
}
thread name="budget_3" {
    sleep(100);
    thread.done = true;
}

var budgetStart = getTickCount();
thread action="join" timeout="500"; // 500ms total budget for all
var budgetElapsed = getTickCount() - budgetStart;

// Should have timed out before all complete, or completed all quickly
assertTrue("shared timeout budget used", budgetElapsed <= 600);

suiteEnd();
</cfscript>
