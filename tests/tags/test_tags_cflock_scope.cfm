<cfscript>
// <cflock> / lock{} scope= isolation, name/scope exclusivity, and throwOnTimeout.
//
// Before v0.553.0 the runtime read only name=/type=/timeout=: `scope=` was
// discarded, so every scope lock fell back to the single name "default" and
// unrelated scopes (and unrelated applications) serialized against each other;
// throwOnTimeout= was discarded too, so a contended lock always threw.
//
// Every expectation here was probed against Lucee 7.0.4 (the reference engine):
// isolation between scopes, `type="lock"` on both errors, and Lucee's exact
// message wording.
suiteBegin( "cflock scope and throwOnTimeout" );

// --- name= and scope= are mutually exclusive -------------------------------
assertThrows( "cflock: name= and scope= together is an error", function() {
    lock name="excl_probe" scope="application" timeout="1" { }
} );

// --- distinct scopes must not contend -------------------------------------
// Two threads hold the application- and server-scope locks; a request-scope
// acquire must not wait behind either of them.
function scopeIsolation() {
    thread name="lkScopeApp" { lock scope="application" timeout="20" { sleep( 1200 ); } }
    thread name="lkScopeSrv" { lock scope="server"      timeout="20" { sleep( 1200 ); } }
    sleep( 200 );
    elapsed = getTickCount();
    lock scope="request" timeout="10" { ran = true; }
    elapsed = getTickCount() - elapsed;
    thread action="join" name="lkScopeApp";
    thread action="join" name="lkScopeSrv";
    return elapsed;
}
assertTrue(
      "cflock scope=request is not blocked by held application/server scope locks"
    , scopeIsolation() < 600
);

// --- throwOnTimeout -------------------------------------------------------
// One thread holds each lock for longer than the contending 1s timeout, so both
// acquires below genuinely time out.
function throwOnTimeoutBehaviour() {
    result = {};
    thread name="lkTotFalse" { lock name="tot_false" timeout="20" { sleep( 4000 ); } }
    thread name="lkTotTrue"  { lock name="tot_true"  timeout="20" { sleep( 4000 ); } }
    sleep( 200 );

    // throwOnTimeout="false": no error, and the body is skipped.
    result.bodyRan = false;
    lock name="tot_false" timeout="1" throwOnTimeout="false" { result.bodyRan = true; }

    // Default (true): a `lock`-typed exception, worded as Lucee words it.
    try {
        lock name="tot_true" timeout="1" { }
        result.errType = "NO ERROR";
        result.errMsg  = "";
    } catch ( any e ) {
        result.errType = e.type;
        result.errMsg  = e.message;
    }

    thread action="join" name="lkTotFalse";
    thread action="join" name="lkTotTrue";
    return result;
}
totResult = throwOnTimeoutBehaviour();
assertFalse( "cflock throwOnTimeout=false skips the body instead of throwing", totResult.bodyRan );
assert( "cflock timeout raises a lock-typed exception", totResult.errType, "lock" );
assert(
      "cflock timeout message matches Lucee"
    , totResult.errMsg
    , "a timeout occurred after 1 second trying to acquire a exclusive lock with name [tot_true]."
);

// --- nesting different scopes must not self-deadlock ----------------------
nested = "";
lock scope="application" timeout="10" {
    lock scope="server" timeout="10" {
        lock scope="request" timeout="10" {
            lock name="nested_named" timeout="10" { nested = "all four"; }
        }
    }
}
assert( "nested locks across different scopes do not deadlock", nested, "all four" );

// A same-scope re-entry is still reentrant (it must not wait on itself).
reentered = "";
lock scope="application" timeout="10" {
    lock scope="application" timeout="10" { reentered = "yes"; }
}
assert( "re-entering the same scope lock is reentrant", reentered, "yes" );

suiteEnd();
</cfscript>
