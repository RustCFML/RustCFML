<!---
  GitHub #249: java.lang.System property shim defects.
  1. setProperty() nulled the variable holding the receiver (the shim's set*
     method matched the implicit-setter rule, so the CallMethod op wrote the
     null result back over the receiver var).
  2. The written value was discarded (no process-global store).
  3. getProperty(unsetKey) / getenv(unsetVar) returned "" instead of null,
     flipping isNull() save/restore guards.

  Fix: process-global property map shared by all System shims; setProperty
  stores + returns prior (null if none) and never touches the receiver;
  getProperty/getenv return null for unset keys; the shim-over-nonshim
  write-back guard stops any shim set*-method result clobbering the receiver.
--->
<cfscript>
suiteBegin("java.lang.System property shim (GitHub 249)");

// 1. setProperty must NOT null the receiver variable.
sys = createObject("java", "java.lang.System");
sys.setProperty("probe.key.249", "hello");
assert("setProperty does not null the receiver", isNull(sys), false);
assert("receiver still usable after setProperty", isNull(sys.getProperty("probe.key.249")), false);

// 1b. Same for a var-scoped receiver inside a function.
function insideFn() {
    var s = createObject("java", "java.lang.System");
    s.setProperty("k.249", "v");
    return isNull(s);
}
assert("var-scoped receiver not nulled inside function", insideFn(), false);

// 2. The written value persists and is visible through a FRESH System object.
fresh = createObject("java", "java.lang.System");
assert("value persists across System instances", fresh.getProperty("probe.key.249"), "hello");

// setProperty returns the PREVIOUS value (or null if none).
assert("setProperty returns null when no prior value",
       isNull(createObject("java", "java.lang.System").setProperty("brand.new.249", "x")), true);
assert("setProperty returns prior value",
       fresh.setProperty("probe.key.249", "world"), "hello");
assert("re-read reflects the new value", fresh.getProperty("probe.key.249"), "world");

// 3. getProperty(unset) is null, not "".
u = fresh.getProperty("definitely.not.set.anywhere.249");
assert("unset getProperty is null", isNull(u), true);

// 2-arg getProperty(key, default) returns the default for an unset key.
assert("getProperty default arg", fresh.getProperty("still.unset.249", "DEF"), "DEF");

// getenv(unset) is null, not "".
assert("getenv unset is null", isNull(fresh.getenv("DEFINITELY_UNSET_ENV_249")), true);

// Built-in property fallbacks still resolve.
assert("file.separator resolves", len(fresh.getProperty("file.separator")), 1);

// The composite finally-style save/restore pattern our specs use must not throw.
threw = false;
try {
    sys3 = createObject("java", "java.lang.System");
    prior = sys3.getProperty("wheels.testClient.baseUrl.249");
    sys3.setProperty("wheels.testClient.baseUrl.249", "http://example:1234");
    if (isNull(prior)) {
        sys3.clearProperty("wheels.testClient.baseUrl.249");
    } else {
        sys3.setProperty("wheels.testClient.baseUrl.249", prior);
    }
} catch (any e) {
    threw = true;
}
assert("finally-style save/restore does not throw", threw, false);
assert("clearProperty cleared the key",
       isNull(createObject("java","java.lang.System").getProperty("wheels.testClient.baseUrl.249")), true);

suiteEnd();
</cfscript>
