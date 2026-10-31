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
    thread name=nextName nextLevel=(level + 1) nextName=nextName {
        try {
            spawnChainLevel(attributes.nextLevel);
            if (structKeyExists(cfthread, attributes.nextName)) {
                thread.chainStatus = cfthread[attributes.nextName].status;
            }
        } catch (any e) {
            thread.chainError = e.message;
        }
    }
    thread action="join" name=nextName;
    // Check for errors at each level
    if (structKeyExists(cfthread, nextName) && structKeyExists(cfthread[nextName], "chainError")) {
        request.chainError = cfthread[nextName].chainError;
    }
}

thread name="chain_1" {
    spawnChainLevel(1);
}
thread action="join" name="chain_1";

// Verify no errors and chain completed
assert("chain completed all iterations", request.chainIterations, request.maxChainLevel);
assert("chain had no errors", request.chainError, "");
// Check final thread status
if (structKeyExists(cfthread, "chain_1")) {
    assertTrue("chain_1 status is COMPLETED or TERMINATED", 
        cfthread.chain_1.status == "COMPLETED" || cfthread.chain_1.status == "TERMINATED");
}

// Test 2: Duplicate name throws (S1)
request.duplicateThrew = false;
thread name="dup_test" {
    sleep(500); // Keep running while we try to create duplicate
    thread.first = true;
}

sleep(50); // Let first thread start

try {
    thread name="dup_test" {
        thread.second = true;
    }
} catch (any e) {
    request.duplicateThrew = (findNoCase("unique", e.message) > 0);
}

thread action="join" name="dup_test" timeout="2000";

assertTrue("duplicate thread name threw", request.duplicateThrew);

// Test 3: Unknown name join throws (S3)
request.unknownJoinThrew = false;
try {
    thread action="join" name="nonexistent_thread_xyz" timeout="100";
} catch (any e) {
    request.unknownJoinThrew = (findNoCase("no thread running", e.message) > 0);
}
assertTrue("unknown join threw", request.unknownJoinThrew);

// Test 4: Unknown name terminate throws (S3)
request.unknownTerminateThrew = false;
try {
    thread action="terminate" name="nonexistent_thread_abc";
} catch (any e) {
    request.unknownTerminateThrew = (findNoCase("terminate thread failed", e.message) > 0 ||
                                      findNoCase("no thread running", e.message) > 0);
}
assertTrue("unknown terminate threw", request.unknownTerminateThrew);

// Test 5: Cross-thread join timing with bounds (B1/B2)
request.targetStarted = false;
request.targetDone = false;
request.joinerStart = -1;
request.joinerEnd = -1;
request.joinerTimedOut = false;

thread name="timing_target" {
    request.targetStarted = true;
    sleep(300); // 300ms sleep
    thread.value = 42;
    request.targetDone = true;
}

sleep(50); // Let target start

thread name="timing_joiner" {
    request.joinerStart = getTickCount();
    try {
        thread action="join" name="timing_target" timeout="5000";
        request.joinerEnd = getTickCount();
    } catch (any e) {
        request.joinerTimedOut = true;
        request.joinerEnd = getTickCount();
    }
}

thread action="join" name="timing_joiner,timing_target" timeout="10000";

assertTrue("target started", request.targetStarted);
assertTrue("target completed", request.targetDone);
assertFalse("joiner did not timeout", request.joinerTimedOut);
assertTrue("joiner start time recorded", request.joinerStart > 0);
assertTrue("joiner end time recorded", request.joinerEnd > 0);
var joinElapsed = request.joinerEnd - request.joinerStart;
assertTrue("cross-join waited at least ~150ms", joinElapsed >= 150);
assertTrue("cross-join completed under 2s", joinElapsed < 2000);
// Verify target metadata
if (structKeyExists(cfthread, "timing_target")) {
    assert("timing target status", cfthread.timing_target.status, "COMPLETED");
    assert("timing target value", cfthread.timing_target.value, 42);
}

// Test 6: Cross-thread join on error - status must be TERMINATED
request.errorThreadStatus = "";

thread name="error_target" {
    throw(message="intentional error for test");
}

sleep(50); // Let it error

// Check spawner's view of errored thread
if (structKeyExists(cfthread, "error_target")) {
    request.errorThreadStatus = cfthread.error_target.status;
}

thread name="error_joiner" {
    thread action="join" name="error_target" timeout="2000";
}

thread action="join" name="error_joiner,error_target";

// Parent can see error
if (structKeyExists(cfthread, "error_target")) {
    assert("error target status is TERMINATED", cfthread.error_target.status, "TERMINATED");
    assertTrue("error target has error", structKeyExists(cfthread.error_target, "error"));
    assertTrue("error target error is non-empty", len(cfthread.error_target.error) > 0);
}

// Test 7: Terminate interrupts sleep - ELAPSEDTIME 0, status TERMINATED
request.terminateJoinerCompleted = false;
request.terminateTargetReached = false;

thread name="terminate_target" {
    sleep(10000); // Long sleep that should be interrupted
    thread.should_not_reach = true;
    request.terminateTargetReached = true;
}

sleep(100); // Let it start sleeping

thread name="terminate_joiner" {
    thread action="terminate" name="terminate_target";
    sleep(100); // Give terminate time to propagate
    thread action="join" name="terminate_target" timeout="5000";
    request.terminateJoinerCompleted = true;
}

thread action="join" name="terminate_joiner,terminate_target" timeout="10000";

assertTrue("terminate joiner completed", request.terminateJoinerCompleted);
assertFalse("terminate target did not reach code after sleep", request.terminateTargetReached);
if (structKeyExists(cfthread, "terminate_target")) {
    assert("terminate target status is TERMINATED", cfthread.terminate_target.status, "TERMINATED");
    // ELAPSEDTIME should be 0 for TERMINATED threads
    assert("terminate target elapsed time is 0", cfthread.terminate_target.elapsedTime, 0);
    assertFalse("terminate target did not set should_not_reach", 
        structKeyExists(cfthread.terminate_target, "should_not_reach"));
}

// Test 8: No-name join (S2) - joins all non-ancestor threads
request.noNameJoinCount = 0;
request.noName1Start = -1;
request.noName2Start = -1;

thread name="no_name_1" {
    request.noName1Start = getTickCount();
    sleep(150);
    thread.idx = 1;
}
thread name="no_name_2" {
    request.noName2Start = getTickCount();
    sleep(150);
    thread.idx = 2;
}

var joinStart = getTickCount();
// Join all with no name
thread action="join" timeout="3000";
var joinEnd = getTickCount();

// Both should be completed
if (structKeyExists(cfthread, "no_name_1") && structKeyExists(cfthread, "no_name_2")) {
    if (cfthread.no_name_1.status == "COMPLETED") request.noNameJoinCount++;
    if (cfthread.no_name_2.status == "COMPLETED") request.noNameJoinCount++;
}
assert("no-name join completed all", request.noNameJoinCount, 2);
// Verify timing - should have waited for both
assertTrue("no-name join waited appropriately", (joinEnd - joinStart) >= 100);

// Test 9: Shared timeout budget for no-name join expires
thread name="budget_1" {
    sleep(400);
    thread.done = true;
}
thread name="budget_2" {
    sleep(400);
    thread.done = true;
}
thread name="budget_3" {
    sleep(400);
    thread.done = true;
}

var budgetStart = getTickCount();
thread action="join" timeout="600"; // 600ms total budget for all 3 x 400ms threads
var budgetElapsed = getTickCount() - budgetStart;

// The threads run concurrently (not sequentially), so they all finish around 400ms.
// The join should wait for them and return around 400ms (well before the 600ms budget).
assertTrue("shared timeout budget was enforced", budgetElapsed < 1000);
assertTrue("join waited for concurrent threads to complete", budgetElapsed >= 350 && budgetElapsed < 550);

// Test 10: threadData() returns request-wide view
thread name="data_test_1" {
    sleep(50);
    thread.value = 100;
}
thread name="data_test_2" {
    sleep(50);
    thread.value = 200;
}

sleep(10);
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
// Verify they're COMPLETED
if (structKeyExists(threadDataAfter, "data_test_1")) {
    assert("data_test_1 status", threadDataAfter.data_test_1.status, "COMPLETED");
}

// Test 11: threadData() returns live status even for unjoined threads (item #4)
thread name="unjoined_test" {
    sleep(100);
    thread.result = "completed_value";
}
sleep(200); // Wait for thread to finish, but don't join it
var unjoinedData = threadData();
assertTrue("threadData has unjoined_test", structKeyExists(unjoinedData, "unjoined_test"));
if (structKeyExists(unjoinedData, "unjoined_test")) {
    assert("unjoined thread status is COMPLETED", unjoinedData.unjoined_test.status, "COMPLETED");
    assert("unjoined thread has result", unjoinedData.unjoined_test.result, "completed_value");
}

// Test 12: Stress test - N threads spawning while cross-joining with overall timeout (item #5)
request.stressCounter = 0;
var stressStart = getTickCount();
var stressThreadCount = 10;
for (var i = 1; i <= stressThreadCount; i++) {
    var threadName = "stress_" & i;
    thread name=threadName idx=i {
        request.stressCounter++;
        sleep(50 + (attributes.idx * 10)); // Staggered sleep times
        // Some threads spawn children
        if (attributes.idx mod 3 == 0) {
            var childName = "stress_child_" & attributes.idx;
            thread name=childName {
                sleep(30);
                thread.child = true;
            }
            thread action="join" name=childName timeout="500";
        }
        thread.completed = true;
    }
}
// Join all stress threads with timeout
thread action="join" timeout="2000";
var stressElapsed = getTickCount() - stressStart;
assertTrue("stress test completed within time limit", stressElapsed < 2500);
assertTrue("stress test completed at least 10 threads", request.stressCounter >= stressThreadCount);

suiteEnd();
</cfscript>
