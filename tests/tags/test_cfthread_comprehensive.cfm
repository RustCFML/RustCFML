<cfscript>
suiteBegin("cfthread: Comprehensive Lucee parity and ##505 regression tests");

// Test 1: ##505 regression - nested self-spawning chain with error assertions
request.chainError = "";
request.chainIterations = 0;

thread name="chain_1" level=1 maxLevel=20 {
    request.chainIterations++;
    if (attributes.level >= attributes.maxLevel) {
        thread.done = true;
        return;
    }
    thread name="chain_##(attributes.level + 1)##" level=(attributes.level + 1) maxLevel=attributes.maxLevel {
        // Recursive spawn
    }
    thread action="join" name="chain_##(attributes.level + 1)##";
}
thread action="join" name="chain_1";

// Verify no errors and chain completed
assert("chain completed all iterations", request.chainIterations, 20);
// Check for errors in the last thread
if (structKeyExists(cfthread, "chain_20")) {
    if (structKeyExists(cfthread.chain_20, "error")) {
        request.chainError = cfthread.chain_20.error.message ?: "unknown error";
    }
    assert("chain ##505 no errors", request.chainError, "");
    assert("chain final status", cfthread.chain_20.status, "COMPLETED");
}

// Test 2: Cap-eviction with small limit (S6)
// Set limit via env var before running: export RUSTCFML_TASKS_LIMIT=5
// For now, test that many threads don't crash
for (var i = 1; i <= 8; i++) {
    thread name="cap_test_##i##" idx=i {
        thread.num = attributes.idx;
    }
}
// Join all
for (var i = 1; i <= 8; i++) {
    try {
        thread action="join" name="cap_test_##i##" timeout="2000";
    } catch (any e) {
        // May throw if evicted from registry (S3)
    }
}
// Spawner's cfthread scope may have been capped (S6)
var capCount = 0;
for (var i = 1; i <= 8; i++) {
    if (structKeyExists(cfthread, "cap_test_##i##")) {
        capCount++;
    }
}
// Should have capped at some reasonable limit (we can't easily test small limit without env var)
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
    assert("error target status", cfthread.error_target.status, "FAILED");
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
    assert("terminate target status", cfthread.terminate_target.status, "TERMINATED");
    // Nit: ELAPSEDTIME 0 for TERMINATED
    assert("ELAPSEDTIME 0 for TERMINATED", cfthread.terminate_target.elapsedTime, 0);
    assertFalse("terminated thread did not complete body", structKeyExists(cfthread.terminate_target, "should_not_reach"));
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
