<cfscript>
suiteBegin("cfthread: nested self-spawning chain has bounded cfthread scope (GH ##505)");

// Before the fix (GH ##505), a thread started from inside another thread
// shared its parent's `cfthread` scope struct (not a copy), and
// `store_cfthread_running` added each new thread's metadata to it.
// A self-rescheduling chain therefore grew ~1.3 KB per thread forever.
//
// This test verifies:
// 1. Each thread gets its own independent cfthread scope
// 2. A nested chain doesn't accumulate all previous threads' metadata
// 3. Threads can still access their own metadata normally

// Test 1: Simple nested case - child has its own scope
request.childScopeSize = 0;

thread name="parent1" {
	thread name="child1" {
		// Child sees its own cfthread scope, not parent's
		request.childScopeSize = structCount( cfthread );
	}
	thread action="join" name="child1";
}
thread action="join" name="parent1";

// Child should have a small bounded scope
assertTrue("child has bounded scope", request.childScopeSize <= 2);

// Test 2: Chain of 3 levels to show scope doesn't accumulate
request.level1Size = 0;
request.level2Size = 0;
request.level3Size = 0;

thread name="level1" {
	request.level1Size = structCount( cfthread );
	thread name="level2" {
		request.level2Size = structCount( cfthread );
		thread name="level3" {
			request.level3Size = structCount( cfthread );
		}
		thread action="join" name="level3";
	}
	thread action="join" name="level2";
}
thread action="join" name="level1";

// Before the fix: sizes would be [1, 2, 3] (accumulating)
// After the fix: each level has its own small scope
assertTrue("level 1 bounded", request.level1Size <= 2);
assertTrue("level 2 bounded", request.level2Size <= 2);  
assertTrue("level 3 bounded", request.level3Size <= 2);

// The key property: level 3 doesn't see level 1's entries
// So the scope size doesn't grow with nesting depth
var maxSize = max( max( request.level1Size, request.level2Size ), request.level3Size );
assertTrue("scope doesn't accumulate across nesting", maxSize < 5);

// Test 3: Parent can still observe directly-spawned threads after join
thread name="observable" {
	thread.result = 42;
}
thread action="join" name="observable";

assertTrue("parent sees its own thread", structKeyExists( cfthread, "observable" ));
assert("thread metadata preserved", cfthread.observable.result, 42);

// Test 4: Siblings don't see each other (regression check for GH ##234)
request.s1Sees2 = false;
request.s2Sees1 = false;

thread name="sib1" {
	sleep( 50 );
	request.s1Sees2 = structKeyExists( cfthread, "sib2" );
}
thread name="sib2" {
	sleep( 50 );
	request.s2Sees1 = structKeyExists( cfthread, "sib1" );
}
thread action="join" name="sib1,sib2";

assertFalse("sibling 1 doesn't see sibling 2", request.s1Sees2);
assertFalse("sibling 2 doesn't see sibling 1", request.s2Sees1);

suiteEnd();
</cfscript>
