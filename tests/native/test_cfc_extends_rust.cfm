<cfscript>
// Exercises a CFC inheriting from a Rust-registered class via
// `extends="rust:Counter"`. Skipped unless RUSTCFML_NATIVE_SMOKE_TEST=1
// (same smoke gate as test_native_class.cfm) because the Counter class
// only registers under that flag.
smokeFlag = "";
try { smokeFlag = createObject("java", "java.lang.System").getenv("RUSTCFML_NATIVE_SMOKE_TEST"); } catch (any e) {}
if (isNull(smokeFlag)) smokeFlag = "";
if (smokeFlag != "1") {
    suiteBegin("CFC extends rust: (skipped — set RUSTCFML_NATIVE_SMOKE_TEST=1 to run)");
    suiteEnd();
    return;
}

suiteBegin("CFC extends rust: — construction + dispatch");

// Construction: a CFC declaring extends="rust:Counter" gets a default-
// constructed parent attached under __super.
inst = createObject("component", "oop.native_cfcs.counter_child");
assertTrue("instance is a struct", isStruct(inst));
assertTrue("instance has __super after construction", structKeyExists(inst, "__super"));

// Unknown rust class on a CFC parent must error at construction.
threw = false;
try {
    createObject("component", "oop.native_cfcs.counter_bad_parent");
} catch (any e) {
    threw = true;
}
assertTrue("unknown rust parent class throws at construction", threw);

// super.X dispatches to the Rust parent (default-constructed Counter at 0).
assert("super.get() on fresh instance is 0", inst.bumpTwice(), 2);
assert("CFC override of add() calls super.add(n*2)", inst.add(5), 12);

// Implicit fall-through: CFC doesn't define `increment`, so inst.increment()
// should reach the Rust parent's method.
fresh = createObject("component", "oop.native_cfcs.counter_child");
assert("implicit fall-through to parent.increment", fresh.increment(), 1);
assert("implicit fall-through to parent.get",       fresh.get(),       1);

// Each child gets its own parent instance — no shared state.
a = createObject("component", "oop.native_cfcs.counter_child");
b = createObject("component", "oop.native_cfcs.counter_child");
a.increment();
assert("instance a parent state",   a.get(), 1);
assert("instance b parent untouched", b.get(), 0);

// isInstanceOf recognises the rust: parent name.
assertTrue("isInstanceOf(inst, 'rust:Counter') is true", isInstanceOf(inst, "rust:Counter"));
assertFalse("isInstanceOf(inst, 'rust:Other') is false", isInstanceOf(inst, "rust:Other"));

// getMetadata surfaces the rust: parent under extends.name.
md = getMetadata(inst);
assert("getMetadata extends.name carries rust: prefix", md.extends.name, "rust:Counter");

// Explicit super(args) replaces the default-constructed parent.
seeded = createObject("component", "oop.native_cfcs.counter_seeded").init(42);
assert("super(args) seeded parent state", seeded.get(), 42);
assert("seeded parent still dispatches add()", seeded.add(8), 50);

// A separately-constructed seeded instance is independent.
other = createObject("component", "oop.native_cfcs.counter_seeded").init(7);
assert("second seeded instance has its own parent", other.get(), 7);
assert("first seeded instance unaffected", seeded.get(), 50);

// Property fall-through: CFC has no `value` field but parent exposes one
// via get_property/set_property — reads and writes route through the trait.
propInst = createObject("component", "oop.native_cfcs.counter_seeded").init(100);
assert("read this.value falls through to parent", propInst.value, 100);
propInst.value = 250;
assert("write this.value routes to parent.set_property", propInst.value, 250);
assert("parent state visible via super.get()",       propInst.get(),    250);

// Unknown native properties fall back to the CFC struct.
propInst.cfcOnly = "hello";
assert("unknown property writes land on the CFC struct", propInst.cfcOnly, "hello");

// --- `new X()` path -------------------------------------------------------
// The flyweight (component-instance) build produces a flyweight Instance for
// `new X()` (createObject stays a marker for now). The native parent must be
// stored PER-INSTANCE on the Instance — not on the shared class blueprint —
// so each instance has independent parent state and method/property
// fall-through still reaches the Rust parent. Passes on the marker build too.
n = new oop.native_cfcs.counter_child();
assert("new: implicit fall-through to parent.increment", n.increment(), 1);
assert("new: implicit fall-through to parent.get",       n.get(),       1);
assert("new: super.X dispatch (bumpTwice)",              n.bumpTwice(),  3);
assert("new: CFC override wrapping super.add",           n.add(5),       13);

// Per-instance independence — the crux of the per-instance-parent fix.
na = new oop.native_cfcs.counter_child();
nb = new oop.native_cfcs.counter_child();
na.increment();
assert("new: instance na parent state",     na.get(), 1);
assert("new: instance nb parent untouched", nb.get(), 0);

// super(args) in init() reconstructs the parent per-instance.
nseed = new oop.native_cfcs.counter_seeded(42);
assert("new: super(args) seeded parent state",  nseed.get(), 42);
assert("new: seeded parent still dispatches add()", nseed.add(8), 50);
nseed2 = new oop.native_cfcs.counter_seeded(7);
assert("new: second seeded instance independent", nseed2.get(), 7);
assert("new: first seeded instance unaffected",   nseed.get(),  50);

// Property fall-through to the native parent (read + write).
nprop = new oop.native_cfcs.counter_seeded(100);
assert("new: read this.value falls through to parent", nprop.value, 100);
nprop.value = 250;
assert("new: write this.value routes to parent.set_property", nprop.value, 250);
assert("new: parent state visible via super.get()",   nprop.get(),  250);
nprop.cfcOnly = "hi";
assert("new: unknown property writes land on the CFC", nprop.cfcOnly, "hi");

suiteEnd();
</cfscript>
