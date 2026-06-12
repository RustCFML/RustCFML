<cfscript>
suiteBegin("Core: assigning a null/void function return must NOT create the target key");

// Background (cross-engine contract):
// Assigning the result of a function that returns null/void (`return;` or no
// return statement at all) must NOT create the target variable. On Lucee the
// assigned name stays undefined — StructKeyExists() is false, isDefined() is
// false — in every scope (local, var, variables, unscoped), and assigning a
// void return to a PRE-EXISTING variable deletes it. Only isNull() on a read
// of the name answers true. RustCFML 0.130.0 instead materializes the key
// with a null value:
//
//   function v() { return; }
//   local.rv = v();
//   StructKeyExists(local, "rv")   RustCFML 0.130.0 -> TRUE   Lucee -> FALSE
//   isDefined("rv")                RustCFML 0.130.0 -> TRUE   Lucee -> FALSE
//   isNull(local.rv)               both -> TRUE (so isNull alone can't discriminate)
//
// This is the OTHER half of the old $callback() breaker (the caller-local-leak
// half was fixed in v0.118 / PR #93). Wheels' universal invocation pattern is
//
//   local.rv = $invoke(...);
//   if (!StructKeyExists(local, "rv")) { local.rv = true; }
//
// i.e. "callback returned nothing -> treat as true". When the engine
// materializes a null-valued key, the default-true fallback never fires, the
// pattern returns null, every model callback chain evaluates falsy, and
// save() aborts before the INSERT — silently. (Deliberately NOT asserted:
// StructKeyList(local) — Lucee itself lists the name there even though
// StructKeyExists is false; that quirk is not part of the contract Wheels
// relies on.)

// --- helpers ---
function nrkVoid() {
	return; // explicit bare return
}
function nrkNoReturn() {
	var x = 1; // no return statement at all (implicit void)
}

// --- CONTROL (green on both engines): the call itself reads as null ---
assertTrue("CONTROL: isNull() on the void call itself is true", isNull(nrkVoid()));

// --- the gap: local-scope target inside a function ---
function nrkLocalProbe() {
	local.rv = nrkVoid();
	return {
		keyExists = structKeyExists(local, "rv"),
		defined   = isDefined("rv"),
		nullRead  = isNull(local.rv)
	};
}
nrkShape = nrkLocalProbe();
assertFalse("local.rv = voidFn(): StructKeyExists(local,'rv') is false", nrkShape.keyExists);
assertFalse("local.rv = voidFn(): isDefined('rv') is false", nrkShape.defined);
assertTrue("CONTROL: isNull(local.rv) reads true after the void assignment", nrkShape.nullRead);

// --- var-declared target inside a function ---
function nrkVarProbe() {
	var rv2 = nrkVoid();
	return structKeyExists(local, "rv2");
}
assertFalse("var rv2 = voidFn(): no 'rv2' key lands in local", nrkVarProbe());

// --- variables-scope target at template level ---
variables.nrkTv = nrkVoid();
assertFalse("variables.x = voidFn(): StructKeyExists(variables, x) is false",
	structKeyExists(variables, "nrkTv"));
assertFalse("variables.x = voidFn(): isDefined(x) is false", isDefined("nrkTv"));

// --- unscoped target at template level ---
nrkTv2 = nrkVoid();
assertFalse("unscoped x = voidFn(): isDefined(x) is false", isDefined("nrkTv2"));
assertFalse("unscoped x = voidFn(): no key lands in variables",
	structKeyExists(variables, "nrkTv2"));

// --- a PRE-EXISTING variable assigned a void return is DELETED ---
variables.nrkPre = "before";
variables.nrkPre = nrkVoid();
assertFalse("pre-existing variables-scope var is deleted by the void assignment",
	structKeyExists(variables, "nrkPre"));
function nrkPreLocalProbe() {
	local.pre = "before";
	local.pre = nrkVoid();
	return structKeyExists(local, "pre");
}
assertFalse("pre-existing local is deleted by the void assignment", nrkPreLocalProbe());

// --- a function with NO return statement behaves identically ---
function nrkNoReturnProbe() {
	local.rv = nrkNoReturn();
	return structKeyExists(local, "rv");
}
assertFalse("fn with no return statement: assignment creates no key either",
	nrkNoReturnProbe());

// --- the Wheels $invoke()/callback shape: default-true fallback must fire ---
function nrkCallbackChain() {
	local.rv = nrkVoid();
	if (!structKeyExists(local, "rv")) {
		local.rv = true;
	}
	return isNull(local.rv) ? "NULL" : toString(local.rv);
}
assert("Wheels callback pattern: default-true fallback fires for a void return",
	nrkCallbackChain(), "true");

// --- CONTROL (green on both engines): a value-returning fn DOES create the key ---
function nrkValueFn() {
	return "val";
}
function nrkValueProbe() {
	local.rv = nrkValueFn();
	return structKeyExists(local, "rv") ? local.rv : "KEY-MISSING";
}
assert("CONTROL: value-returning fn assignment creates the key", nrkValueProbe(), "val");

suiteEnd();
</cfscript>