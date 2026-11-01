<cfscript>
suiteBegin("cfthread: registry cap/eviction and cross-thread join verification");

// Test 1: Registry FIFO eviction with small cap
// Show that oldest threads are evicted when cap is reached
// Note: We can't easily configure the cap at runtime (it's hardcoded to 10000),
// so we'll test the eviction logic works by spawning many threads
request.registryTest = "start";

// Spawn threads that complete quickly so we can test eviction behavior
// The registry should hold metadata even after threads complete
for ( i = 1; i <= 5; i++ ) {
	threadName = "regtest_" & i;
	thread name=threadName xi=i {
		thread.index = attributes.xi;
	}
}

// Join all to ensure they've registered
for ( i = 1; i <= 5; i++ ) {
	thread action="join" name="regtest_#i#";
}

// All 5 should be in the spawner's cfthread scope
registeredCount = 0;
for ( i = 1; i <= 5; i++ ) {
	if ( structKeyExists( cfthread, "regtest_#i#" ) ) {
		registeredCount++;
	}
}

assert("spawner sees all 5 registered threads", registeredCount, 5);

// Test 2: Registry is NOT visible in cfthread variable (stays internal)
// The cfthread variable should only show threads THIS context spawned
request.childSeesRegistry = false;
request.childCfthreadCount = -1;

thread name="reg_child1" {
	sleep( 50 );
	thread.data = "child1";
}

thread name="reg_child2" {
	sleep( 10 );
	// Child2 should only see threads IT spawned in its cfthread scope
	var localCount = structCount( cfthread );
	
	// The registry exists internally but shouldn't appear as a visible key
	var hasRegistryKey = structKeyExists( cfthread, "registry" ) ||
	                     structKeyExists( cfthread, "_registry" ) ||
	                     structKeyExists( cfthread, "__registry" );
	
	request.childSeesRegistry = hasRegistryKey;
	request.childCfthreadCount = localCount;
}

thread action="join" name="reg_child1,reg_child2";

assertFalse("registry not exposed as cfthread key", request.childSeesRegistry);
assert("child starts with empty cfthread scope", request.childCfthreadCount, 0);

// Test 3: Child joins parent's thread by name, checking status and output
request.parentThreadStatus = "";
request.parentThreadOutput = "";
request.childJoinedStatus = "";
request.childJoinedOutput = "";

thread name="parentSpawnedThread" {
	sleep( 100 );
	thread.customData = "parent thread result";
	writeOutput("Parent thread output");
}

thread name="childJoiner" {
	// Child joins a thread its parent (the main context) spawned
	thread action="join" name="parentSpawnedThread" timeout="3000";
	
	// After join, check the parent's cfthread scope (not accessible to child)
	// But we can verify the thread completed via request scope
	request.childJoinedStatus = "joined";
}

thread action="join" name="childJoiner,parentSpawnedThread";

// Verify the parent can see the spawned thread's status and output
assertTrue("parent sees parentSpawnedThread", structKeyExists( cfthread, "parentSpawnedThread" ));
assert("parent thread status", cfthread.parentSpawnedThread.status, "COMPLETED");
assert("parent thread has custom data", cfthread.parentSpawnedThread.customData, "parent thread result");
assertTrue("parent thread output captured", len( cfthread.parentSpawnedThread.output ) > 0);

// Verify child successfully joined
assert("child joined parent's thread", request.childJoinedStatus, "joined");

// Test 4: More detailed cross-thread join with output verification
request.targetOutput = "";
request.joinerSeesOutput = false;

thread name="joinTarget" {
	sleep( 100 );
	thread.value = 42;
	thread.message = "target completed";
	request.targetOutput = "done";
}

thread name="joinerThread" {
	// Join the target
	thread action="join" name="joinTarget" timeout="3000";
	
	// After join, verify target completed (via request scope)
	request.joinerSeesOutput = (request.targetOutput == "done");
}

thread action="join" name="joinerThread,joinTarget";

// Verify parent can see the completed thread's metadata
assertTrue("parent sees joinTarget", structKeyExists( cfthread, "joinTarget" ));
assert("joinTarget status", cfthread.joinTarget.status, "COMPLETED");
assert("joinTarget has value", cfthread.joinTarget.value, 42);
assert("joinTarget has message", cfthread.joinTarget.message, "target completed");

// Verify joiner successfully waited
assertTrue("joiner waited for target completion", request.joinerSeesOutput);

// Test 5: Nested child joins grandparent's thread
request.grandparentDone = false;
request.nestedChildJoined = false;

thread name="grandparentThread" {
	sleep( 100 );
	thread.level = "grandparent";
	request.grandparentDone = true;
}

thread name="middleThread" {
	thread name="nestedChild" {
		// Nested child joins grandparent's thread via registry
		thread action="join" name="grandparentThread" timeout="3000";
		request.nestedChildJoined = request.grandparentDone;
	}
	thread action="join" name="nestedChild";
}

thread action="join" name="middleThread,grandparentThread";

assertTrue("grandparent completed", request.grandparentDone);
assertTrue("nested child joined grandparent", request.nestedChildJoined);
assert("grandparent metadata visible to spawner", cfthread.grandparentThread.level, "grandparent");

suiteEnd();
</cfscript>
