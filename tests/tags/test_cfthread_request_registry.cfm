<cfscript>
suiteBegin("cfthread: request-wide registry (Lucee parity, GH ##505)");

// RustCFML semantics (matching Lucee where possible):
// - Child thread gets empty cfthread scope at START (no parent entries)
// - Threads register in TWO places:
//   1. Spawner's own cfthread scope
//   2. Request-wide registry (shared by all threads in request)
// - terminate/interrupt use request registry (work across threads)
// - Note: join only works for threads this context spawned (Rust limitation)
// - Registry is capped with FIFO eviction (default 10000)

// Test 1: Child doesn't see parent's threads in its cfthread scope
request.childSawParent = false;

thread name="parent1" {
	thread.parentData = "parent";
}
thread action="join" name="parent1";

thread name="child1" {
	// Child should NOT see parent1 in its cfthread scope
	request.childSawParent = structKeyExists( cfthread, "parent1" );
}
thread action="join" name="child1";

assertFalse("child doesn't see parent's thread in cfthread scope", request.childSawParent);

// Test 2: Parent can still observe directly-spawned threads after join
thread name="observable" {
	thread.result = 42;
}
thread action="join" name="observable";

assertTrue("parent sees directly-spawned thread", structKeyExists( cfthread, "observable" ));
assert("thread metadata preserved", cfthread.observable.result, 42);

// Test 3: Siblings don't see each other in their cfthread scopes
request.sibASeesSibB = false;
request.sibBSeesSibA = false;

thread name="sibA" {
	sleep( 50 );
	request.sibASeesSibB = structKeyExists( cfthread, "sibB" );
}
thread name="sibB" {
	sleep( 50 );
	request.sibBSeesSibA = structKeyExists( cfthread, "sibA" );
}
thread action="join" name="sibA,sibB";

assertFalse("sibling A doesn't see sibling B in cfthread scope", request.sibASeesSibB);
assertFalse("sibling B doesn't see sibling A in cfthread scope", request.sibBSeesSibA);

// Test 4: threadTerminate works across threads (via request registry)
request.termTargetStarted = false;
request.termTargetFinished = false;

thread name="termTarget" {
	request.termTargetStarted = true;
	// Loop that should be terminated
	for ( var i = 1; i <= 100; i++ ) {
		sleep( 50 );
		if ( i == 100 ) {
			request.termTargetFinished = true;
		}
	}
}

sleep( 100 ); // Let termTarget start
thread action="terminate" name="termTarget";
thread action="join" name="termTarget" timeout="3000";

assertTrue("terminate target started", request.termTargetStarted);
assertFalse("terminate target did not finish (was terminated)", request.termTargetFinished);

// Test 5: Terminate works from a sibling thread (via request registry)
request.term2Started = false;
request.term2Finished = false;
request.terminatorRan = false;

thread name="term2Target" {
	request.term2Started = true;
	for ( var i = 1; i <= 100; i++ ) {
		sleep( 50 );
		if ( i == 100 ) {
			request.term2Finished = true;
		}
	}
}

thread name="term2inator" {
	sleep( 100 ); // Let term2Target start
	// Terminate from a sibling
	thread action="terminate" name="term2Target";
	request.terminatorRan = true;
}

thread action="join" name="term2inator";
thread action="join" name="term2Target" timeout="3000";

assertTrue("term2 target started", request.term2Started);
assertFalse("term2 target did not finish (was terminated by sibling)", request.term2Finished);
assertTrue("terminator ran", request.terminatorRan);

// Test 6: Memory stays bounded in nested self-spawning chains
// (The original GH ##505 fix: chains don't accumulate entries)
request.chainParentCount = 0;
request.chainChildCount = 0;
request.chainGrandchildCount = 0;

thread name="chainParent" {
	// Parent sees only itself (and any it spawns)
	request.chainParentCount = structCount( cfthread );
	
	thread name="chainChild" {
		// Child sees only threads IT spawns (not parent)
		var beforeSpawn = structCount( cfthread );
		
		thread name="chainGrandchild" {
			// Grandchild sees only threads IT spawns
			request.chainGrandchildCount = structCount( cfthread );
		}
		thread action="join" name="chainGrandchild";
		
		var afterSpawn = structCount( cfthread );
		// Child saw 0 before spawning, then saw its own child
		request.chainChildCount = beforeSpawn;
	}
	thread action="join" name="chainChild";
}
thread action="join" name="chainParent";

// Each level doesn't see ancestors (just itself + its own children)
assertTrue("parent sees bounded threads", request.chainParentCount <= 2);
assert("child doesn't see parent at start", request.chainChildCount, 0);
assert("grandchild doesn't see ancestors", request.chainGrandchildCount, 0);

suiteEnd();
</cfscript>
