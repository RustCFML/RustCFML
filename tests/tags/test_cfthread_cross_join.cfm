<cfscript>
suiteBegin("cfthread: cross-thread join (Lucee parity, GH ##505)");

// RustCFML implements Lucee's dual-registration:
// - Each thread gets its own independent cfthread scope (prevents GH ##505 leak)
// - Threads also register in request-wide registry (enables cross-thread operations)
// - join/terminate work via registry when thread not in local scope

// Test 1: Sibling threads can join each other (cross-thread join)
request.sib2Done = false;
request.sib1Done = false;

thread name="sib2_top" {
	sleep( 100 );
	request.sib2Done = true;
}

thread name="sib1_top" {
	// Join sibling via request registry
	thread action="join" name="sib2_top" timeout="3000";
	request.sib1Done = true;
}

thread action="join" name="sib1_top,sib2_top";

assertTrue("sibling 2 completed", request.sib2Done);
assertTrue("sibling 1 completed and joined sibling 2", request.sib1Done);

// Test 2: Nested siblings can join each other
request.nested2Done = false;
request.nested1Joined = false;

thread name="parent" {
	thread name="nested2" {
		sleep( 100 );
		request.nested2Done = true;
	}
	
	thread name="nested1" {
		thread action="join" name="nested2" timeout="3000";
		request.nested1Joined = true;
	}
	
	thread action="join" name="nested1,nested2";
}
thread action="join" name="parent";

assertTrue("nested sibling 2 completed", request.nested2Done);
assertTrue("nested sibling 1 joined sibling 2", request.nested1Joined);

// Test 3: threadTerminate works across threads
request.termStarted = false;
request.termFinished = false;

thread name="termTarget" {
	request.termStarted = true;
	for ( var i = 1; i <= 100; i++ ) {
		sleep( 50 );
		if ( i == 100 ) {
			request.termFinished = true;
		}
	}
}

sleep( 100 );
thread action="terminate" name="termTarget";
thread action="join" name="termTarget" timeout="3000";

assertTrue("terminate target started", request.termStarted);
assertFalse("terminate target was terminated", request.termFinished);

// Test 4: Memory stays bounded (GH ##505 fix)
request.chainCount = 0;

thread name="chain1" {
	request.chainCount = structCount( cfthread );
	thread name="chain2" {
		thread name="chain3" {
			// Deep nesting
		}
		thread action="join" name="chain3";
	}
	thread action="join" name="chain2";
}
thread action="join" name="chain1";

assertTrue("chain cfthread scope stays bounded", request.chainCount <= 2);

// Test 5: Parent observes directly-spawned threads
thread name="direct" {
	thread.result = 42;
}
thread action="join" name="direct";

assertTrue("parent sees directly-spawned thread", structKeyExists( cfthread, "direct" ));
assert("thread metadata preserved", cfthread.direct.result, 42);

suiteEnd();
</cfscript>
