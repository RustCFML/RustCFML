<cfscript>
suiteBegin("Core: isInstanceOf component base type 'Component'");

// ============================================================
// Lucee parity (probed live against Lucee 6): every CFC is an instance of the
// base type "Component" (case-insensitive) — and ONLY that exact name. It is
// NOT an instance of "Object" nor "lucee.runtime.Component", and a plain struct
// is NOT a "Component". MockBox's normalizeArguments() relies on this to pick
// serializeJSON(cfc) over a member cfc.toString() call (which both engines
// refuse on a bare CFC); without it ~40 cfflow specs errored with
// "Component [X] has no function with name [toString]".
// ============================================================

comp = new NoAccessorComponent();

assert("cfc is a 'Component'", isInstanceOf(comp, "Component"), true);
assert("cfc is a 'component' (case-insensitive)", isInstanceOf(comp, "component"), true);
assert("cfc is NOT an 'Object'", isInstanceOf(comp, "Object"), false);
assert("cfc is NOT 'lucee.runtime.Component'", isInstanceOf(comp, "lucee.runtime.Component"), false);
assert("plain struct is NOT a 'Component'", isInstanceOf({ a = 1 }, "Component"), false);

suiteEnd();
</cfscript>
