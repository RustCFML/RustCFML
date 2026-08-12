<cfscript>
suiteBegin( "cflock timeout semantics" );

/*
 * Verified against Lucee 7.0.4.34 (CommandBox, same tests/runner.cfm), 2026-08-12.
 *
 * WHY `LockOperation` MATTERS BEYOND SPEC-COMPLIANCE: Preside's Bootstrap.cfc
 * guards its application-reload lock with
 *     catch( any e ) { if ( ( e.lockOperation ?: "" ) == "Timeout" ) { 503; abort; } ... }
 * Without the member we took the WRONG branch, which clears
 * `application._preside_reloading` while another request is still mid-reload and
 * then rethrows. With it, Preside serves its intended "still reloading" 503.
 *
 * ⚠️ NOT COVERED HERE — see docs/known-issues.md §40: `timeout="0"` and an omitted
 * `timeout` mean "wait indefinitely" in Lucee but fail/expire here. The fix is
 * small and a 15-assertion version of this suite is ready, but it must land only
 * AFTER the application-scope publication fix — fail-fast currently masks that
 * bug, and making lock losers wait turns 8 concurrent cold Preside requests from 1
 * framework boot into 8. Asserting the Lucee behaviour now would make this suite
 * red; asserting our behaviour would bake a divergence into the tests. So the
 * divergence lives in known-issues, not in here.
 */

// Runs `body` while a background thread holds `lockName` exclusively for
// `holdMs`, and reports how long the foreground acquisition took.
function withHeldLock( required numeric holdMs, required any body ) {
    local.lockName = "cflock_sem_" & createUUID();
    local.holder   = createUUID();

    thread name=local.holder ln=local.lockName holdMs=arguments.holdMs {
        lock name=attributes.ln type="exclusive" timeout=30 {
            sleep( attributes.holdMs );
        }
    }
    sleep( 400 ); // let the thread actually take it

    local.start  = getTickCount();
    local.result = arguments.body( local.lockName );
    local.result.elapsedMs = getTickCount() - local.start;

    thread action="join" name=local.holder timeout=30000;
    return local.result;
}

// --- a positive timeout times out, carrying Lucee's exception members ---------
r = withHeldLock( 3000, function( lockName ) {
    var out = { got=false, caught=false, type="", lockOperation="", hasLockName=false, message="" };
    try {
        lock name=arguments.lockName type="exclusive" timeout=1 { out.got = true; }
    } catch ( any e ) {
        out.caught        = true;
        out.type          = e.type ?: "";
        out.lockOperation = e.lockOperation ?: "(missing)";
        out.hasLockName   = structKeyExists( e, "lockName" );
        out.message       = e.message ?: "";
    }
    return out;
} );
assertFalse( "timeout=1 against a 3s holder does not acquire", r.got );
assertTrue( "timeout=1 throws", r.caught );
assert( "the exception is lock-typed", r.type, "lock" );
assert( "carries LockOperation=Timeout (Preside branches on this)", r.lockOperation, "Timeout" );
assertFalse( "does NOT carry lockName (Lucee does not either)", r.hasLockName );
assertTrue( "message uses Lucee's wording",
            r.message contains "a timeout occurred after 1 second trying to acquire a exclusive lock" );
assertTrue( "waited for roughly its timeout, not 0ms", r.elapsedMs >= 900 );

// --- throwontimeout="false" suppresses the throw and skips the body -----------
r = withHeldLock( 3000, function( lockName ) {
    var out = { got=false, caught=false };
    try {
        lock name=arguments.lockName type="exclusive" timeout=1 throwontimeout=false { out.got = true; }
    } catch ( any e ) {
        out.caught = true;
    }
    return out;
} );
assertFalse( "throwontimeout=false skips the body on timeout", r.got );
assertFalse( "throwontimeout=false does not throw", r.caught );

// --- an UNCONTENDED lock still acquires and runs its body ---------------------
uncontended = { ran = false };
lock name="cflock_sem_free_#createUUID()#" type="exclusive" timeout=5 {
    uncontended.ran = true;
}
assertTrue( "an uncontended exclusive lock runs its body", uncontended.ran );

// --- a readonly lock is not blocked by another readonly holder ---------------
r = withHeldLock( 1500, function( lockName ) {
    var out = { got=false, caught=false };
    try {
        lock name=arguments.lockName type="readonly" timeout=5 { out.got = true; }
    } catch ( any e ) {
        out.caught = true;
    }
    return out;
} );
assertTrue( "readonly acquires once the exclusive holder releases", r.got );
assertFalse( "readonly does not throw", r.caught );

suiteEnd();
</cfscript>
