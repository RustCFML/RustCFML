<!--- A frame's `arguments` scope is its OWN, never the caller's (2026-08-22).

      A frame that takes the lazy-`arguments` path (Lever A: no `arguments` scope
      struct is built when the body provably cannot observe one) used to inherit
      the CALLER's `__arguments_scope` handle through the parent copy-in. Because
      a CfmlStruct is an Arc handle, every downstream site that treated the key as
      "my arguments" was reading — and in StoreLocal's param->arguments sync,
      WRITING — the caller's scope. So assigning to your own unsupplied parameter
      published it into the caller's frame, where the next callee inherited it as
      its own parameter.

      All four assertions below are VERIFIED against Lucee 7.0.5 (CommandBox
      lucee@7): cases 1-3 are the divergence, case 4 is the control that must NOT
      change — a plain unscoped write in a classic-localMode page function DOES
      propagate to the caller, and the fix must distinguish the two. --->
<cfscript>
suiteBegin("Parameter scope ownership");

// --- 1. a lazy frame writing its own unsupplied param must not publish it ---
function ownsParamWrite( numeric n ) { n = 7; return n; }
function readsSameParamName( numeric n ) { return isNull( n ) ? "absent" : n; }
ownsParamWrite();
assert("unsupplied param written on a lazy frame does not reach the next callee",
       readsSameParamName(), "absent");

// --- 2. ...nor the caller's own frame ---
function ownsParamWrite2( numeric zz9 ) { zz9 = 7; return zz9; }
ownsParamWrite2();
callerSawIt = true;
try { callerSawIt = ( zz9 == 7 ); } catch (any e) { callerSawIt = false; }
assertFalse("unsupplied param written on a lazy frame is not readable in the caller",
            callerSawIt);

// --- 3. the same shape with a DEFAULT (eager today, lazy if the default
//        preamble ever stops touching the arguments scope) ---
function hasDefault( numeric m = "7" ) { return m; }
function readsSameDefaultName( numeric m ) { return isNull( m ) ? "absent" : m; }
hasDefault();
assert("an applied default does not reach the next callee",
       readsSameDefaultName(), "absent");

// --- 4. CONTROL: classic-localMode propagation is unchanged. A bare write to a
//        name that is NOT a parameter still reaches the caller. ---
function plainUnscopedWrite() { qq7 = 99; return 1; }
function readsPlainName() { return isNull( qq7 ) ? "absent" : qq7; }
plainUnscopedWrite();
assert("a plain unscoped write still propagates to the caller (classic localMode)",
       readsPlainName(), 99);

// --- 5. an EAGER frame still sees its own applied default on `arguments` ---
function eagerDefault( numeric k = 5 ) { return arguments.k; }
assert("an applied default is visible on the frame's own arguments scope",
       eagerDefault(), 5);
assert("a supplied argument still wins over the default", eagerDefault( 11 ), 11);

// --- 6. a supplied param mutated in the body stays local to the frame ---
function mutatesSupplied( numeric p ) { p = p + 1; return p; }
function readsSuppliedName( numeric p ) { return isNull( p ) ? "absent" : p; }
assert("mutated supplied param returns the new value", mutatesSupplied( 1 ), 2);
assert("mutated supplied param does not reach the next callee",
       readsSuppliedName(), "absent");

suiteEnd();
</cfscript>
