<cfscript>
suiteBegin("Concurrency: a named exclusive lock is reentrant within the same request/thread");

// Background: a thread that already holds an exclusive named lock must be able to
// re-acquire the SAME named lock without blocking — named locks are reentrant per
// thread. Lucee/ACF/BoxLang all let the inner block run immediately.
//
// RustCFML 0.161.0 is NOT reentrant: the inner re-acquire of an already-held named
// lock blocks until the timeout elapses and then throws a lock-timeout error. The
// inner block never runs.
//
//   lock name="x" { lock name="x" { ... } }   -> Lucee: inner runs; RustCFML 0.161: inner times out + throws
//
// Why it matters: same-name re-entry is a normal CFML pattern (a locked section
// calling a helper that locks the same name). On RustCFML it self-deadlocks until
// the timeout, surfacing as request hangs/errors.

lockTrace = "OUTER_BEFORE ";
lockErr = "";
try {
    lock name="reentLockTest" timeout="1" type="exclusive" {
        lockTrace &= "OUTER_IN ";
        lock name="reentLockTest" timeout="1" type="exclusive" {
            lockTrace &= "INNER_IN ";
        }
        lockTrace &= "OUTER_AFTER";
    }
} catch (any e) {
    lockErr = e.message;
}

// --- CONTROL (green on both engines): the outer lock body executes ---
assertTrue("outer lock body executes", findNoCase("OUTER_IN", lockTrace) GT 0);

// --- the gap: the inner re-acquire of the same named lock must succeed (reentrant) ---
assertTrue("inner re-acquire of the same named lock succeeds (reentrant)",
    findNoCase("INNER_IN", lockTrace) GT 0);
assert("no lock-timeout error on re-entry", lockErr, "");

suiteEnd();
</cfscript>
