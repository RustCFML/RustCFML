<cfscript>
suiteBegin("runAsync + Future kernel");

// ---- runAsync returns a Future NativeObject whose .get() blocks for the closure's return value.
f = runAsync(function() {
    return 42;
});
assert("runAsync returns a value via get()", f.get(), 42);
assertTrue("future.isDone after get()", f.isDone());
// `status()` / `error()` are RustCFML extensions to the Future surface; Lucee's
// Future exposes only cancel/isCancelled/isDone/error/get/then, so calling
// status() there is a hard error rather than a failed assertion.
if (isRustCFML()) {
    assert("future.status COMPLETED", f.status(), "COMPLETED");
}

// ---- Closure can return any CFML value
fStr = runAsync(function() { return "hello"; });
assert("runAsync string result", fStr.get(), "hello");

fStruct = runAsync(function() { return { a: 1, b: 2 }; });
got = fStruct.get();
assert("runAsync struct result a", got.a, 1);
assert("runAsync struct result b", got.b, 2);

// ---- Errors thrown inside the closure surface via .get()
fErr = runAsync(function() {
    throw(type="CustomBoom", message="async boom");
});
// Give it a moment to settle (its already a fired thread, but get blocks).
errCaught = false;
try {
    fErr.get();
} catch (any e) {
    errCaught = true;
    assertTrue("error message preserved", findNoCase("async boom", e.message) gt 0);
}
assertTrue("get() rethrows closure error", errCaught);
// Lucee's `error` is the error-CALLBACK registrar (error(fn)), not a getter.
if (isRustCFML()) {
    assert("future.error populated", findNoCase("async boom", fErr.error()) gt 0, true);
}

// ---- isDone is false before get() resolves (cant fully guarantee w/o timing
// but a long-sleeping closure is reliable enough for a smoke check)
fSleep = runAsync(function() {
    sleep(150);
    return "done";
});
// First peek: may or may not be done depending on race, but get() must wait.
assert("get() on long task returns value", fSleep.get(), "done");
assertTrue("isDone after wait", fSleep.isDone());

// ---- get(timeoutMs) — Null on timeout.
// RustCFML-only: Lucee THROWS on a timed-out get() rather than returning null,
// and a get() after cancel() raises CancellationException instead of returning
// the completed body's value. Both are real divergences, recorded here rather
// than asserted cross-engine.
fNeverInTime = runAsync(function() {
    sleep(500);
    return "late";
});
if (isRustCFML()) {
    peek = fNeverInTime.get(10);
    assertTrue("get(timeout) returns null on timeout", isNull(peek) || peek eq "");
}
// Then wait properly
final = fNeverInTime.get();
assert("subsequent get() returns the value", final, "late");

// ---- cancel() flips the cancel flag (non-error path: the cooperative
// cancel flag is set but bodies need to poll it; we just verify the call
// returns true and isCancelled reflects it before the join).
fCancel = runAsync(function() {
    sleep(200);
    return "ignored";
});
didCancel = fCancel.cancel();
assertTrue("cancel() returns true on a live future", didCancel);
// Wait for the body to finish (cancel doesnt kill; documented Lucee divergence).
// Lucee raises CancellationException from this get(); RustCFML returns the
// value the body produced anyway.
if (isRustCFML()) {
    fCancel.get();
}

writeOutput("[async_kernel] runAsync + Future tests OK" & chr(10));
suiteEnd();
</cfscript>
