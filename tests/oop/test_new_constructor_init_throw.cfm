<cfscript>
suiteBegin("OOP: `new Comp(args)` must propagate an exception thrown by init()");

// Background: the `new Component(args)` constructor sugar is defined to run
// init(args) and return the initialized instance — and if init() throws, that
// exception must propagate to the caller, exactly as the explicit
// createObject("component", "X").init(args) form does. This is what makes
// constructor-guard validation work: a CFC whose init() rejects bad arguments
// can be trusted never to hand back a half-built object.
//
// RustCFML 0.161.0 runs init() under `new X(args)` but SWALLOWS any exception
// it throws and returns the partially-initialized object instead. The explicit
// createObject(...).init(args) path propagates the exception correctly, so the
// divergence is specific to the `new` sugar.
//
//   createObject("component","F").init(windowSeconds=0)  -> throws on BOTH (CONTROL)
//   new NewInitThrowFixture(windowSeconds=0)             -> Lucee: throws; RustCFML 0.161: NO throw (object built)
//
// Why it matters: Wheels uses constructor-arg validation across the framework.
// e.g. wheels.middleware.RateLimiter init() throws Wheels.RateLimiter.
// InvalidConfiguration when windowSeconds<=0 / maxRequests<0; middleware is
// registered as `new wheels.middleware.RateLimiter(maxRequests=..,windowSeconds=..)`.
// On RustCFML the guard never fires through the `new` path, so misconfiguration
// silently yields a broken, half-initialized instance instead of failing fast.

// --- CONTROL (green on both engines): explicit createObject().init() propagates ---
assertThrows("CONTROL: createObject(...).init(windowSeconds=0) propagates the init() exception", function() {
    var c = createObject("component", "oop.NewInitThrowFixture").init(windowSeconds = 0);
});

// --- CONTROL (green on both engines): a VALID arg builds a usable instance via `new` ---
nitOk = new oop.NewInitThrowFixture(windowSeconds = 30);
assert("CONTROL: `new Fixture(windowSeconds=30)` builds a usable instance", nitOk.getWindowSeconds(), 30);

// --- the gap: `new Fixture(badArg)` must propagate the exception from init() ---
assertThrows("`new NewInitThrowFixture(windowSeconds=0)` must propagate the exception thrown by init()", function() {
    var bad = new oop.NewInitThrowFixture(windowSeconds = 0);
});

suiteEnd();
</cfscript>
