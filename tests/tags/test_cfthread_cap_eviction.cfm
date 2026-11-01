<cfscript>
// Test cap eviction with RUSTCFML_TASKS_LIMIT
// This test requires RUSTCFML_TASKS_LIMIT=3 environment variable

suiteBegin("cfthread: Cap eviction (RUSTCFML_TASKS_LIMIT)");

// Check if limit is set for this test
var limitEnv = server.system.environment.RUSTCFML_TASKS_LIMIT ?: "";
if (limitEnv != "3") {
    // Skip if not running with the test limit
    assertTrue("RUSTCFML_TASKS_LIMIT test requires env var set to 3 (skipped)", true);
    suiteEnd();
    return;
}

// Spawn 5 threads with limit of 3 - oldest should be evicted
thread name="cap_1" {
    sleep(50);
    thread.val = 1;
}
thread name="cap_2" {
    sleep(50);
    thread.val = 2;
}
thread name="cap_3" {
    sleep(50);
    thread.val = 3;
}
sleep(20); // Let first 3 register

// These should evict cap_1 and cap_2
thread name="cap_4" {
    sleep(50);
    thread.val = 4;
}
thread name="cap_5" {
    sleep(50);
    thread.val = 5;
}

sleep(100); // Let all finish

// Verify cap_3, cap_4, cap_5 are in cfthread scope
assertTrue("cap_3 in scope", structKeyExists(cfthread, "cap_3"));
assertTrue("cap_4 in scope", structKeyExists(cfthread, "cap_4"));
assertTrue("cap_5 in scope", structKeyExists(cfthread, "cap_5"));

// cap_1 and cap_2 should be evicted (oldest-first FIFO)
var cap1Exists = structKeyExists(cfthread, "cap_1");
var cap2Exists = structKeyExists(cfthread, "cap_2");
assertFalse("cap_1 evicted from local scope", cap1Exists);
assertFalse("cap_2 evicted from local scope", cap2Exists);

// Joining an evicted thread should throw (Lucee parity)
var joinEvictedThrew = false;
try {
    thread action="join" name="cap_1";
} catch (any e) {
    joinEvictedThrew = true;
    assertTrue("join evicted throws with 'no thread running'", findNoCase("no thread running", e.message) > 0);
}
assertTrue("joining evicted thread threw", joinEvictedThrew);

suiteEnd();
</cfscript>
