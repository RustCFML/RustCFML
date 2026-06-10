<cfscript>
suiteBegin("Core: caller's local is invisible to an undeclared callee (absence checks)");

// `local` is per-CALL: every invocation gets a fresh, EMPTY local scope. A
// callee that never declares `local.rv` must not be able to SEE a caller's
// `local.rv` — not via StructKeyExists(local, "rv"), not via isNull(local.rv),
// not via a read. Lucee, Adobe CF, and BoxLang all agree.
//
// This is the READ/absence-check residual of the per-frame `local.X` fix that
// shipped in v0.92.0 (PR #77, test_local_scope_frame_isolation.cfm). The
// DECLARATION side is fixed — a callee's `local.x = ...` no longer clobbers
// the caller — but a callee that never declares the name still sees the
// caller's slot when it only reads or absence-checks:
//
//   function innerFn() { return StructKeyExists(local, "rv"); }
//   function outerFn() { local.rv = false; return innerFn(); }
//   outerFn()
//     RustCFML 0.105.0 -> true  (and local.rv reads the caller's false;
//                                isNull(local.rv) says false)
//     Lucee 5.4.8.2    -> false (fresh empty local; isNull(local.rv) -> true)
//
// Why it matters for Wheels: the framework's pervasive default-true tail
//
//   if (!StructKeyExists(local, "rv")) { local.rv = true; }
//   return local.rv;
//
// ($callback() in vendor/wheels/model/callbacks.cfc; the same absence-check
// on local.rv appears in 8 framework files) read the CALLER's local.rv=false
// through this leak, so every model callback chain "returned false" and
// save() aborted before its INSERT — silently, with no error anywhere.
//
// Covered: the template-level shape, the component-method shape (Wheels
// mixins are methods on one CFC), and a declares-first CONTROL that pins the
// already-fixed #77 contract so the two cannot be conflated.

// --- (1) Template-level: undeclared callee gets a fresh, empty local ---
function absenceLeakInner() {
	var r = {
		seesRv   = StructKeyExists(local, "rv"),
		rvIsNull = isNull(local.rv),
		leaked   = ""
	};
	if (r.seesRv) r.leaked = toString(local.rv);
	return r;
}
function absenceLeakOuter() {
	local.rv = false;
	return absenceLeakInner();
}
shape = absenceLeakOuter();
assertFalse("template: StructKeyExists(local,'rv') is false in undeclared callee", shape.seesRv);
assertTrue("template: isNull(local.rv) is true in undeclared callee", shape.rvIsNull);
assert("template: caller's value is not readable through local.rv", shape.leaked, "");

// --- (2) The exact Wheels default-true tail (the silent save()-killer) ---
function absenceLeakCallbackTail() {
	// verbatim shape of the $callback() tail
	if (!StructKeyExists(local, "rv")) {
		local.rv = true;
	}
	return local.rv;
}
function absenceLeakChainRunner() {
	local.rv = false; // the caller's own accumulator, same conventional name
	return absenceLeakCallbackTail();
}
assertTrue("template: default-true tail returns true (caller's false not inherited)",
	absenceLeakChainRunner());

// --- (3) CONTROL: a callee that DECLARES local.rv first owns its slot and
//     the caller keeps hers — the v0.92.0 / PR #77 contract. Green on both
//     engines; guards the wiring and pins that THIS test is about reads. ---
function absenceLeakDeclaredInner() {
	local.rv = "CALLEE";
	return "sees=" & StructKeyExists(local, "rv") & "|val=" & local.rv;
}
function absenceLeakDeclaredOuter() {
	local.rv = false;
	var got = absenceLeakDeclaredInner();
	return got & "|callerRv=" & toString(local.rv);
}
assert("CONTROL: declaring callee sees its own value; caller's survives",
	absenceLeakDeclaredOuter(), "sees=true|val=CALLEE|callerRv=false");

// --- (4) Component-method shape: caller and callee are methods on the same
//     CFC, exactly how Wheels mixins run ($invokeMethod -> $callback) ---
o = createObject("component", "AbsenceLeakFixture");
cshape = o.outerProbe();
assertFalse("component: StructKeyExists(local,'rv') is false in undeclared callee method", cshape.seesRv);
assertTrue("component: isNull(local.rv) is true in undeclared callee method", cshape.rvIsNull);
assert("component: caller's value is not readable through local.rv", cshape.leaked, "");
assertTrue("component: default-true tail returns true across methods",
	o.invokeWithAccumulator());

suiteEnd();
</cfscript>
