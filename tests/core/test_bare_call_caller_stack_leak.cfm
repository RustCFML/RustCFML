<cfscript>
suiteBegin("Core: bare-name method calls ignore caller-stack frames (no dynamic scoping)");

// Background
// ----------
// Inside a CFC method, a function called by BARE NAME resolves against the
// method's own frame and then the component's variables/this scope — NEVER
// against the arguments/locals of ANCESTOR frames on the call stack. CFML is
// lexically scoped: a caller's parameter or local that happens to share a
// name with a component method is pure data and must stay invisible to a
// callee's function-name resolution. Lucee 5/6/7, Adobe CF, and BoxLang all
// agree (all six probe calls succeed on Lucee 5.4.8.2).
//
// RustCFML 0.108.0 resolves bare-name calls through the caller stack
// (dynamic scoping): if ANY ancestor frame holds a non-function value under
// the function's name, the callee's bare call throws "Variable is not a
// function or function '<unknown>' is not defined". The shadowing cascades
// through the WHOLE stack — a grandparent's param breaks a grandchild's
// call. Locals shadow too, not just params. `variables.fn()` and `this.fn()`
// are immune on 0.108.0 (fix-shape hint: bare-name resolution should consult
// the same scopes, functions-before-ancestor-data, frame-isolated).
//
// Likely the same frame-fusion family as the per-frame `local` gaps
// (test_local_scope_frame_isolation.cfm and the absence-leak test filed
// alongside this one): frames are not isolated, so name lookups fall through
// to ancestor frames. Fixing frame-isolated name resolution may collapse
// several of these at once.
//
// Surfaced booting Wheels: the $invokeOnSelf trampoline takes a param named
// `$args`, and it sits above every controller action. Every bare `$args(...)`
// call in ANY framework function below it on the stack — redirectTo()'s URL
// building among them — hit the leak and died "Variable is not a function":
// every POST redirect 500'd. Latent landmine: any param/local name matching
// any function called bare anywhere below it on the stack.

bclProbe = createObject("component", "core.BareCallLeakProbe");

// (1) CONTROL — no shadowing ancestor frame: bare calls resolve. Green on
//     both engines; guards the wiring.
bclDirect = bclProbe.bclTarget();
assert("direct: bare bclFnA() resolves with no shadowing frame",
	bclDirect.bareA & bclDirect.bareAError, "FN_A_RESULT");
assert("direct: bare bclFnB() resolves with no shadowing frame",
	bclDirect.bareB & bclDirect.bareBError, "FN_B_RESULT");

// (2) THE GAP — the IMMEDIATE CALLER declares a struct param named bclFnA.
//     (On failure the assert shows the thrown message instead of the value.)
bclMid = bclProbe.viaMid();
assert("via mid: caller's struct param bclFnA does not shadow bare bclFnA()",
	bclMid.bareA & bclMid.bareAError, "FN_A_RESULT");
assert("via mid: unshadowed bare bclFnB() still resolves",
	bclMid.bareB & bclMid.bareBError, "FN_B_RESULT");

// (3) THE CASCADE — GRANDPARENT shadows bclFnB, parent shadows bclFnA:
//     both bare calls in the grandchild must still resolve.
bclDeep = bclProbe.viaDeep();
assert("via deep: parent's struct param bclFnA does not shadow bare bclFnA()",
	bclDeep.bareA & bclDeep.bareAError, "FN_A_RESULT");
assert("via deep: grandparent's struct param bclFnB does not shadow bare bclFnB()",
	bclDeep.bareB & bclDeep.bareBError, "FN_B_RESULT");

// (4) LOCALS shadow too — the caller holds `var bclFnA = {...}`.
bclLocalRes = bclProbe.viaLocalShadow();
assert("via local: caller's var bclFnA does not shadow bare bclFnA()",
	bclLocalRes.bareA & bclLocalRes.bareAError, "FN_A_RESULT");

// (5) CONTROLS — scoped calls are immune even under the fully-shadowed
//     stack (green on both engines, including RustCFML 0.108.0).
assert("via deep: variables.bclFnA() resolves under the shadowed stack",
	bclDeep.scopedA & bclDeep.scopedAError, "FN_A_RESULT");
assert("via deep: this.bclFnA() resolves under the shadowed stack",
	bclDeep.thisA & bclDeep.thisAError, "FN_A_RESULT");

suiteEnd();
</cfscript>
