<cfscript>
suiteBegin("_schedule (one-shot + periodic)");

// `_schedule` is a RustCFML async-kernel BIF with no Lucee/ACF equivalent, so
// the cross-engine run skips the whole suite rather than spraying reds.
hasSchedule = true;
try {
    _schedule(function() { return 1; }, 0).get();
} catch (any e) {
    hasSchedule = false;
}
if (!hasSchedule) {
    assertTrue("_schedule unsupported on this engine - suite skipped", true);
    writeOutput("[async_kernel] _schedule unsupported here; skipped" & chr(10));
    suiteEnd();
} else {

// ---- delay=0: schedule fires immediately and resolves like runAsync.
f0 = _schedule(function() { return "now"; }, 0);
assert("_schedule(delay=0) returns value", f0.get(), "now");
assertTrue("schedule future isDone", f0.isDone());

// ---- delay>0: fires after the delay; we measure the elapsed wallclock to
// confirm we actually waited.
start = getTickCount();
fDelay = _schedule(function() { return getTickCount(); }, 80);
fireTick = fDelay.get();
elapsed = getTickCount() - start;
assertTrue("delayed schedule waited >= 50ms", elapsed gte 50);
assertTrue("delayed schedule produced a numeric tick", isNumeric(fireTick));

// ---- Struct options form
fOpts = _schedule(function() { return 7; }, { delayMs: 0 });
assert("_schedule({delayMs:0}) returns value", fOpts.get(), 7);

// ---- cancel before fire: cooperative cancel during the sleep window
fCancelMe = _schedule(function() { return "should not run"; }, 1000);
didCancel = fCancelMe.cancel();
assertTrue("schedule cancel returns true", didCancel);
// Give the relay a beat to see the flag and post TERMINATED.
res = fCancelMe.get(2000);
assert("cancelled schedule -> status TERMINATED", fCancelMe.status(), "TERMINATED");

// ---- Periodic: everyMs is FIXED-RATE (period measured from each run's start),
// spacedMs is FIXED-DELAY (measured from each run's end). Both used to be
// parsed and dropped, so the closure fired exactly ONCE (GH #314).
//
// The count is polled up to a generous deadline rather than sampled after a
// fixed sleep: the first fire of a cold serve-mode request pays the child-VM
// setup once, which alone can outlast a 1.5s window (observed — the cold run
// saw 0 fires where the warm run saw 7). Polling keeps the assertion about
// "does it re-fire at roughly this rate", not about machine speed.
periodicCount = function(opts, targetFires, deadlineMs) {
    var counterFile = getTempDirectory() & "/rustcfml_sched_#createUUID()#.txt";
    fileWrite(counterFile, "0");
    var body = function() {
        fileWrite(counterFile, toString(val(fileRead(counterFile)) + 1));
    };
    var fut = _schedule(body, opts);
    var deadline = getTickCount() + deadlineMs;
    var n = 0;
    while (getTickCount() lt deadline) {
        n = val(fileRead(counterFile));
        if (n gte targetFires) { break; }
        sleep(50);
    }
    fut.cancel();
    n = val(fileRead(counterFile));
    fileDelete(counterFile);
    return n;
};

// 5 fires at a 200ms period is ~1s of work; 10s of headroom absorbs a cold
// start. Falling short means it is not re-firing at all.
everyFires = periodicCount({ delayMs: 100, everyMs: 200 }, 5, 10000);
assertTrue("everyMs re-fires (>1), got #everyFires#", everyFires gt 1);
assertTrue("everyMs reaches 5 fires within 10s, got #everyFires#", everyFires gte 5);

spacedFires = periodicCount({ delayMs: 100, spacedMs: 200 }, 5, 10000);
assertTrue("spacedMs re-fires (>1), got #spacedFires#", spacedFires gt 1);
assertTrue("spacedMs reaches 5 fires within 10s, got #spacedFires#", spacedFires gte 5);

// The period must actually be RESPECTED — a broken driver that re-fires with no
// wait would blow past this bound long before the poll loop above returned.
// 200ms x 5 fires cannot complete in under ~800ms of elapsed wall-clock.
rateStart = getTickCount();
rateFires = periodicCount({ delayMs: 0, everyMs: 200 }, 5, 10000);
rateElapsed = getTickCount() - rateStart;
assertTrue("everyMs honours the period (5 fires took #rateElapsed#ms)", rateFires lt 5 || rateElapsed gte 700);

// Poll `counterFile` until it reaches `want`, or the deadline passes. Returns
// the count actually reached — same cold-start reasoning as periodicCount.
waitForFires = function(counterFile, want, deadlineMs) {
    var deadline = getTickCount() + deadlineMs;
    var n = 0;
    while (getTickCount() lt deadline) {
        n = val(fileRead(counterFile));
        if (n gte want) { return n; }
        sleep(50);
    }
    return val(fileRead(counterFile));
};

// ---- A periodic schedule stops on cancel() and fires no more.
cancelFile = getTempDirectory() & "/rustcfml_sched_cancel_#createUUID()#.txt";
fileWrite(cancelFile, "0");
fPeriodic = _schedule(function() {
    fileWrite(cancelFile, toString(val(fileRead(cancelFile)) + 1));
}, { delayMs: 50, everyMs: 100 });
atCancel = waitForFires(cancelFile, 2, 10000);
fPeriodic.cancel();
assertTrue("periodic fired before cancel, got #atCancel#", atCancel gt 1);
// Re-read AFTER the cancel: a run already in flight may still land, so settle
// first, then prove the count is stable across a further 6 periods.
sleep(300);
settled = val(fileRead(cancelFile));
sleep(600);
assert("cancel() stops the schedule", val(fileRead(cancelFile)), settled);
fileDelete(cancelFile);

// ---- A body that throws is NOT rescheduled (ScheduledExecutorService parity):
// otherwise a permanently-broken heartbeat spins forever.
throwFile = getTempDirectory() & "/rustcfml_sched_throw_#createUUID()#.txt";
fileWrite(throwFile, "0");
fThrow = _schedule(function() {
    fileWrite(throwFile, toString(val(fileRead(throwFile)) + 1));
    throw(type = "SchedBoom", message = "boom");
}, { delayMs: 50, everyMs: 100 });
assert("throwing periodic body runs at least once", waitForFires(throwFile, 1, 10000), 1);
sleep(600);  // 6 more periods
assert("throwing periodic body is not rescheduled", val(fileRead(throwFile)), 1);
fThrow.cancel();
fileDelete(throwFile);

// ---- delayMs still one-shot when no period is given (no regression).
onceFile = getTempDirectory() & "/rustcfml_sched_once_#createUUID()#.txt";
fileWrite(onceFile, "0");
_schedule(function() {
    fileWrite(onceFile, toString(val(fileRead(onceFile)) + 1));
}, { delayMs: 100 });
assert("delayMs alone fires", waitForFires(onceFile, 1, 10000), 1);
sleep(600);
assert("delayMs alone fires exactly once", val(fileRead(onceFile)), 1);
fileDelete(onceFile);

writeOutput("[async_kernel] _schedule tests OK" & chr(10));
suiteEnd();
}
</cfscript>
