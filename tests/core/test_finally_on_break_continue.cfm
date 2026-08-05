<cfscript>
suiteBegin("try/finally runs on break and continue");

// A `break` or `continue` jumps to the loop exit/next iteration at runtime, and
// the jump does not run the `finally` bodies it escapes. `return` and `rethrow`
// already emitted those finallys inline; break/continue did not, so a
// `try {} finally {}` (or anything lowered to one — `lock {}`,
// `transaction {}`) was skipped entirely when the body broke out of a loop.
//
// Surfaced as GH #308: a `break` out of a `transaction { }` ran neither the
// commit nor the rollback, leaving the transaction open for the rest of the
// request. Verified against Lucee 7.0.4, which runs the finally in every case
// below.

// --- break out of a try/finally inside a loop ---
log = [];
for ( i = 1; i <= 3; i++ ) {
	try {
		arrayAppend( log, "body#i#" );
		break;
	} finally {
		arrayAppend( log, "finally#i#" );
	}
}
assert("finally runs when the try body breaks", arrayToList( log ), "body1,finally1");

// --- continue out of a try/finally inside a loop ---
log = [];
for ( i = 1; i <= 2; i++ ) {
	try {
		continue;
	} finally {
		arrayAppend( log, "finally#i#" );
	}
}
assert("finally runs on every continue", arrayToList( log ), "finally1,finally2");

// --- only the finallys INSIDE the loop run, and only once ---
log = [];
try {
	for ( i = 1; i <= 3; i++ ) {
		try {
			break;
		} finally {
			arrayAppend( log, "inner" );
		}
	}
	arrayAppend( log, "after-loop" );
} finally {
	arrayAppend( log, "outer" );
}
assert("break runs the inner finally, then continues past the loop", arrayToList( log ), "inner,after-loop,outer");

// --- nested finallys between the break and its loop run innermost-first ---
log = [];
for ( i = 1; i <= 2; i++ ) {
	try {
		try {
			break;
		} finally {
			arrayAppend( log, "inner" );
		}
	} finally {
		arrayAppend( log, "middle" );
	}
}
assert("nested finallys run innermost first", arrayToList( log ), "inner,middle");

// --- break inside a switch inside a loop only unwinds to the switch ---
log = [];
for ( i = 1; i <= 2; i++ ) {
	try {
		switch ( i ) {
			case 1:
				try { break; } finally { arrayAppend( log, "switch-finally#i#" ); }
			default:
				arrayAppend( log, "default#i#" );
		}
		arrayAppend( log, "after-switch#i#" );
	} finally {
		arrayAppend( log, "loop-finally#i#" );
	}
}
assert(
	"a switch break runs only the finallys inside the switch",
	arrayToList( log ),
	"switch-finally1,after-switch1,loop-finally1,default2,after-switch2,loop-finally2"
);

// --- a lock released by a break can be re-acquired ---
function breakOutOfLock() {
	for ( i = 1; i <= 2; i++ ) {
		lock name="tfob1" type="exclusive" timeout="5" {
			break;
		}
	}
	return "left";
}
breakOutOfLock();
reacquired = false;
lock name="tfob1" type="exclusive" timeout="5" {
	reacquired = true;
}
assertTrue("a lock left by a break is released, not leaked", reacquired);

suiteEnd();
</cfscript>
