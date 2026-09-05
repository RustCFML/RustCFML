<!--- In-function `variables.x` semantics, pinned (2026-08-22).
      Written for the 3B #3 LoadVariablesProperty fusion, which was BUILT, measured
      ZERO on all four real workloads (+-0.5%) — though see docs/inline-caches-plan.md
      on the null floor of that measurement — and REVERTED (it also shrank the
      then-existing JIT's admission; that engine was removed in v0.653.0). The assertions are lowering-independent and
      stay: they pin CFC private-scope reads, method-table fall-through, page-scope UDF
      `variables`, mutation visibility, and the miss -> throw contract — plus the
      Lucee-verified parity note below. --->
<cfscript>
suiteBegin("In-function variables.x semantics");

// --- CFC method reading its private scope (the Preside shape) ---
w = new fixtures.FusedVarsWidget( 7, 35 );
assert("method reads variables.x", w.total(), 42);
assert("variables.x sees a mutation made by an earlier method", w.bumpAndRead(), 43);

// --- method-table fall-through: variables.someMethod resolves the method ---
assert("variables.<method> resolves via the method table", w.callSibling(), "sibling:43");

// VERIFIED PARITY (2026-08-22, Lucee 7.0.5): a method extracted as a VALUE and invoked
// at PAGE scope throws on BOTH engines — Lucee says "key [A] doesn't exist", we say
// "Variable 'a' is undefined" — for both `w.extractSibling()` and `w.sibling`
// extraction. Also byte-identical on RustCFML v0.613.0 (before the 2026-08-22 engine
// rework) and with this fusion on/off. Canonical behaviour, NOT a bug: an extracted
// method value does not carry its instance scope to a foreign call site. The engine's
// binding branch covers the narrower shapes that DO work (e.g. `this.method` handed to
// ColdBox as a filter) and those are asserted elsewhere in the suite.

// --- page-level UDF: `variables` inside a function is the page scope ---
pageVal = "from-page";
function readsPageVariables() {
    return variables.pageVal;
}
assert("page UDF variables.x reads the page scope", readsPageVariables(), "from-page");

// --- miss must THROW (GetProperty contract), not yield null ---
missThrew = false;
function readsMissing() { return variables.noSuchKeyEver_3b3; }
try { readsMissing(); } catch (any e) { missThrew = true; }
assertTrue("variables.<missing> throws inside a function", missThrew);

suiteEnd();
</cfscript>
