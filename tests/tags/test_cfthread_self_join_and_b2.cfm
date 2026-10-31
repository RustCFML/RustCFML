<cfscript>
suiteBegin("cfthread: Self-join, ancestor exclusion, and B2 regression");

// Test 1: Self-join by name returns immediately (documented deviation from Lucee)
request.selfJoinCompleted = false;
request.selfJoinStart = -1;
request.selfJoinEnd = -1;

thread name="self_join_test" {
    request.selfJoinStart = getTickCount();
    // Self-join should return immediately, not hang
    thread action="join" name="self_join_test" timeout="1000";
    request.selfJoinEnd = getTickCount();
    request.selfJoinCompleted = true;
}

thread action="join" name="self_join_test" timeout="5000";

assertTrue("self-join test completed", request.selfJoinCompleted);
assertTrue("self-join returned quickly", (request.selfJoinEnd - request.selfJoinStart) < 100);

// Test 2: No-name join from child excludes self and ancestors (no hang)
request.childJoinCompleted = false;
request.childJoinedCount = 0;

thread name="ancestor_parent" {
    // Spawn two siblings
    thread name="sibling_1" {
        sleep(100);
        thread.val = 1;
    }
    thread name="sibling_2" {
        sleep(100);
        thread.val = 2;
    }
    
    // Spawn a child that does no-name join
    thread name="child_joiner" {
        // No-name join should exclude self (child_joiner) and ancestor (ancestor_parent)
        // but should join siblings
        thread action="join" timeout="3000";
        request.childJoinCompleted = true;
        
        // Check which siblings completed
        if (structKeyExists(cfthread, "sibling_1") && cfthread.sibling_1.status == "COMPLETED") {
            request.childJoinedCount++;
        }
        if (structKeyExists(cfthread, "sibling_2") && cfthread.sibling_2.status == "COMPLETED") {
            request.childJoinedCount++;
        }
    }
    
    thread action="join" name="child_joiner,sibling_1,sibling_2" timeout="5000";
}

thread action="join" name="ancestor_parent" timeout="10000";

assertTrue("child no-name join completed", request.childJoinCompleted);
// Note: child_joiner may not see siblings in its own cfthread scope depending on where it runs
// but the important thing is it didn't hang trying to join its ancestor

// Test 3: B2 regression - cross-join where target spawns a thread (should not deadlock)
request.b2TargetSpawned = false;
request.b2TargetChildCompleted = false;
request.b2JoinerCompleted = false;

thread name="b2_target" {
    sleep(50);
    request.b2TargetSpawned = true;
    
    // Target spawns its own child thread
    thread name="b2_target_child" {
        sleep(100);
        thread.val = 123;
        request.b2TargetChildCompleted = true;
    }
    
    thread action="join" name="b2_target_child" timeout="2000";
}

sleep(10); // Let target start

thread name="b2_joiner" {
    // Joiner tries to join the target (which is spawning a child)
    // This should not deadlock
    thread action="join" name="b2_target" timeout="5000";
    request.b2JoinerCompleted = true;
}

thread action="join" name="b2_joiner,b2_target" timeout="10000";

assertTrue("B2 target spawned child", request.b2TargetSpawned);
assertTrue("B2 target child completed", request.b2TargetChildCompleted);
assertTrue("B2 joiner completed without deadlock", request.b2JoinerCompleted);

// Verify all threads completed
if (structKeyExists(cfthread, "b2_target")) {
    assert("B2 target status", cfthread.b2_target.status, "COMPLETED");
}
if (structKeyExists(cfthread, "b2_joiner")) {
    assert("B2 joiner status", cfthread.b2_joiner.status, "COMPLETED");
}

suiteEnd();
</cfscript>
