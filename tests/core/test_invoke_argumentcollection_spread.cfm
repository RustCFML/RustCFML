<cfscript>
suiteBegin("Core: invoke() spreads an `argumentCollection` key in its argument struct");

// ============================================================
// Background  (companion to test_invoke_canonical_forms.cfm)
// ============================================================
// `argumentCollection` is a SPECIAL key on every CFML argument-passing
// surface. When the argument struct handed to the positional BIF
// invoke(objectOrName, method, argStruct) contains an `argumentCollection`
// key, Lucee 5/6/7 and Adobe ColdFusion spread the INNER struct into the
// callee's arguments scope — the same contract as fn(argumentCollection = st)
// and cfinvoke's argumentCollection attribute. Declared params named by inner
// keys receive those values, params the inner struct does not name keep their
// declared defaults, and the literal key "argumentCollection" itself never
// appears in the callee's arguments scope.
//
// RustCFML 0.100.0 does not special-case the key — the inner struct is
// silently dropped (it neither spreads nor lands as a literal named arg):
//
//   function target(string name = "DEFAULT", string path = "P0") {...}
//   invoke(o, "target", { argumentCollection = { name = "posts" } })
//     Lucee 5.4.8.2    -> "name=posts path=P0"     (inner struct spread)
//     RustCFML 0.100.0 -> "name=DEFAULT path=P0"   (inner struct dropped)
//
// The plain 3-arg struct form (the CONTROL below) already agrees on both
// engines; only the nested-key spread diverges.
//
// Wheels rides this contract on every request: $doubleCheckedLock() forwards
// its executeArgs as `$invoke(method = ..., argumentCollection = executeArgs)`
// (vendor/wheels/Global.cfc:41) and $invoke() pushes that struct through the
// engine's dynamic-invoke machinery (Global.cfc:289-320). Controller class
// resolution ($controller -> $doubleCheckedLock -> $createControllerClass)
// runs on this path, so an engine that drops the key hands the resolver EMPTY
// arguments and silently turns every user controller action into a no-op.
// ============================================================

fixture = createObject("component", "InvokeArgCollTarget");

// --- CONTROL: plain 3-arg struct form binds declared params on BOTH engines ---
// Guards the wiring: if this fails, dynamic invocation itself is broken,
// not the argumentCollection special-casing under test.
assert("CONTROL: plain arg struct binds declared params",
	invoke(fixture, "target", { name = "posts", path = "/x" }),
	"name=posts path=/x");

// --- the gap: a nested argumentCollection key must be SPREAD ---
assert("argumentCollection key is spread; unnamed param keeps its declared default",
	invoke(fixture, "target", { argumentCollection = { name = "posts" } }),
	"name=posts path=P0");

assert("argumentCollection key naming every param binds them all",
	invoke(fixture, "target", { argumentCollection = { name = "posts", path = "/admin" } }),
	"name=posts path=/admin");

assert("spread also applies when invoking by component NAME",
	invoke("InvokeArgCollTarget", "target", { argumentCollection = { name = "posts" } }),
	"name=posts path=P0");

// The spread populates the callee's arguments scope even when the method
// declares no params at all (Lucee reports key "name"; keying on presence,
// not order/case, per the named_args_no_numeric_alias conventions).
spreadKeys = invoke(fixture, "argKeyList", { argumentCollection = { name = "posts" } });
assertTrue("spread reaches the arguments scope of a paramless method",
	listFindNoCase(spreadKeys, "name") gt 0);

// And the special key itself must NOT leak through as a literal argument —
// pins the fix shape: spread the inner struct, don't bind the key by name.
assertFalse("no literal 'argumentCollection' key leaks into the callee",
	invoke(fixture, "hasLiteralArgumentCollectionKey", { argumentCollection = { name = "posts" } }));

suiteEnd();
</cfscript>
