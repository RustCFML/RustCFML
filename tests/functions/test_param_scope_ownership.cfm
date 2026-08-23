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

// --- 7. inherited page vars are NOT part of the callee's `local` view, under
//        ANY casing. The frame's inherited-key set is keyed by the interned
//        case-insensitive `Key`; it used to be a case-SENSITIVE HashSet<String>,
//        so a probe in different casing could miss and mis-classify an inherited
//        page variable as a frame local. Verified identical on Lucee 7.0.5. ---
MyPageVar = "page";
function inheritedNotLocal() {
    return structKeyExists( local, "mypagevar" )
        || structKeyExists( local, "MyPageVar" )
        || structKeyExists( local, "MYPAGEVAR" );
}
assertFalse("an inherited page var is not in `local` under any casing",
            inheritedNotLocal());

function realLocalIsLocal() {
    var RealLocal = 1;
    return structKeyExists( local, "reallocal" ) && structKeyExists( local, "RealLocal" );
}
assertTrue("a genuine var-declared local IS in `local` under any casing",
           realLocalIsLocal());

// --- 8. `var`-declaring a name that was INHERITED reclaims it as a real local
//        (GH #243), and does so case-insensitively. This is the one path that
//        REMOVES from the inherited-key set rather than probing it, so it is the
//        half case 7 cannot cover. Verified on Lucee 7.1.0. ---
Shadowed = "page";
function reclaimsInherited() {
    var shadowed = "mine";
    return structKeyExists( local, "Shadowed" ) && local.SHADOWED == "mine";
}
assertTrue("a var-declared name that was inherited becomes a real local, whatever the casing",
           reclaimsInherited());

// --- 9. ...and being a real local, it must NOT be written back to the caller,
//        WHATEVER the casings. `declared_locals` used to be a case-SENSITIVE
//        HashSet<String> holding the name as written plus its lowercase form --
//        two casings, where three are in play. The write-back loops probe with
//        the key's casing as stored in `locals`, which is the casing the CALLER
//        seeded it with, so `var fileName` failed to shield a caller's
//        `filename` and silently overwrote it on exit. All four verified
//        byte-identical against Lucee 7.1.0. ---
sameCasing = "page";
function declaresSameCasing() { var sameCasing = "mine"; return 1; }
declaresSameCasing();
assert("a var-declared local is not written back to the caller", sameCasing, "page");

DiffCasing = "page";
function declaresDiffCasing() { var diffcasing = "mine"; return 1; }
declaresDiffCasing();
assert("...nor when the declaration's casing differs from the caller's",
       DiffCasing, "page");

MiXeD = "page";
function declaresMixed() { var mIxEd = "mine"; return 1; }
declaresMixed();
assert("...nor for arbitrary mixed casing", MiXeD, "page");

// CONTROL: the fix must not swallow a genuine unscoped write (classic
// localMode), which is the whole reason the write-back loop exists.
function unscopedStillPropagates() { notDeclared8 = 42; return 1; }
unscopedStillPropagates();
assert("a genuine unscoped write still reaches the caller", notDeclared8, 42);

// --- 10. `var` in one casing then assignment in ANOTHER stays local and does
//         not leak to `variables` (ColdBox Router.buildFlashScope). This is what
//         the removed second `declared_locals` entry used to buy; the
//         case-folding set covers it by construction. ---
function declareThenAssignOtherCasing() {
    var flashPath8 = "x";
    flashpath8 = "y";
    return flashPath8;
}
assert("a write in different casing hits the var-declared local, not variables",
       declareThenAssignOtherCasing(), "y");
assertFalse("...and does not leak the name into the caller's scope",
            isDefined("flashpath8"));

suiteEnd();
</cfscript>
