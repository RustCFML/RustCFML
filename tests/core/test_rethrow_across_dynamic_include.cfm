<cfscript>
// An error escaping a DYNAMIC include (`include "#path#"`) inside a `lock {}`
// must surface with its real message. The lock's cleanup arm is a synthesised
// `rethrow`, and the include's error path used to jump into it without setting
// the exception register, so the caller saw "No exception to rethrow" and the
// real error was lost (Wheels onRequest: `lock { include "#arguments.targetPage#"; }`).
suiteBegin("rethrow across a dynamic include inside lock");

request.__dyn_inc_target = "rethrow_dynamic_include_thrower.cfm";
msg = "";
try {
    lock name="rethrowDynIncTest" type="exclusive" timeout="5" {
        include "#request.__dyn_inc_target#";
    }
} catch ( any e ) {
    msg = e.message;
}
assert("the included template's real error message survives the lock cleanup", msg, "boom from the dynamic include");

// Plain try/catch around the dynamic include, no lock.
msg2 = "";
try { include "#request.__dyn_inc_target#"; } catch ( any e ) { msg2 = e.message; }
assert("plain try around a dynamic include keeps the message", msg2, "boom from the dynamic include");

suiteEnd();
</cfscript>
