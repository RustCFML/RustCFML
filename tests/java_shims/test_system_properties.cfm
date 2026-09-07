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

// setProperty returns the PREVIOUS value (or null if none). Clear first so this
// holds even in serve mode, where the process-global store persists across
// requests (a fixed key set on a prior request would otherwise have a prior).
createObject("java", "java.lang.System").clearProperty("brand.new.249");
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

<!---
  GitHub #412: `var osInfo = System.getProperties()` left osInfo UNDEFINED with
  nothing thrown, because the shim had no `getproperties` arm and the resulting
  fall-through dispatch produced a silent null. Two fixes, both asserted below:
  the method now exists and returns a java.util.Properties MAP, and an
  unhandled method on a shimmed class now THROWS instead of yielding null (the
  java-shim instalment of the GH #307 silent-no-op class).

  Also asserted: the three property surfaces agree. They are now built from one
  table in java_shims.rs; before, `System.getProperty` knew five keys where
  `server.system.properties` knew eleven and disagreed with it on two of them.
--->
<cfscript>
suiteBegin("java.lang.System.getProperties and unhandled-shim errors (GitHub 412)");

sysP = createObject("java", "java.lang.System");

// 1. The reported repro: the assignment must actually bind.
function getPropsIsDefined() {
    var osInfo = createObject("java","java.lang.System").getProperties();
    return isDefined("osInfo");
}
assert("getProperties() assigns to a local", getPropsIsDefined(), true);

props = sysP.getProperties();
assert("getProperties returns a struct", isStruct(props), true);

// 2. It is a MAP shim, so java.util.Map members dispatch on the result. A plain
//    struct would fail every one of these.
assert("map get() works", props.get("os.name"), props["os.name"]);
assert("map getProperty() works", props.getProperty("os.name"), props["os.name"]);
assert("map containsKey() works", props.containsKey("user.home"), true);

// 3. os.name is the JVM spelling, not Rust's `std::env::consts::OS`. Real code
//    string-matches the JVM value ("Mac OS", "Windows", "Linux"); "macos" can
//    never match it. Assert the SHAPE rather than a platform, so this holds on
//    every CI runner: the first letter is capitalised on every JVM os.name.
osName = props["os.name"];
assert("os.name is non-empty", len(osName) GT 0, true);
assert("os.name is JVM-cased, not lowercase rust", osName, ucFirst(osName));

// 4. All three surfaces must agree — they are one table now.
assert("getProperties agrees with getProperty", props["os.name"], sysP.getProperty("os.name"));
assert("getProperties agrees with server scope", props["os.name"], server.os.name);
assert("server.system.properties agrees too", props["os.name"], server.system.properties["os.name"]);
assert("file.encoding present in all", props["file.encoding"], server.system.properties["file.encoding"]);

// 5. A runtime setProperty value shows up in the map and wins over a default.
sysP.setProperty("probe.key.412", "visible");
assert("setProperty value appears in getProperties",
       createObject("java","java.lang.System").getProperties()["probe.key.412"], "visible");

// 6. An unhandled method on a SHIMMED class throws rather than returning null.
//    This is the whole point: a silent null is what made #412 surface far from
//    its cause, as "Variable 'osInfo' is undefined" pointing at innocent code.
threwUnhandled = false;
try {
    sysP.totallyBogusMethodName412();
} catch (any e) {
    threwUnhandled = true;
}
assert("unhandled shim method throws", threwUnhandled, true);

// 7. lineSeparator() — a real java.lang.System method that the silent null had
//    been hiding. Same value as the line.separator property.
assert("lineSeparator matches the property",
       sysP.lineSeparator(), sysP.getProperty("line.separator"));

suiteEnd();
</cfscript>
