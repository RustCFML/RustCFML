<cfscript>
suiteBegin("Core: this.-dot method call must not detach the frame's this binding");

// ============================================================
// Background
// ============================================================
// Inside a component method, calling a sibling method with a `this.`-DOT
// qualified call expression (`this.noop()`) must leave the frame's `this`
// binding alone. Lucee 5/6/7, Adobe ColdFusion 2018-2025, and BoxLang all
// dispatch the member call against the live object: any subsequent
// `this.key = value` write in that frame -- or in any frame it calls --
// is visible to the caller after the method returns.
//
// RustCFML 0.108.0 instead DETACHES the frame's `this` binding onto a
// data-complete SHALLOW COPY of the object at the moment of the dot-call.
// Every later this-write in that frame, and in frames called after it
// (any call shape), lands on the detached copy: visible in-frame,
// DISCARDED when the detaching frame returns.
//
//     component {                       // fixture
//         public void function noop() {}
//         public void function combo() {
//             this.noop();              // <- detaches `this` on RustCFML
//             this.mk = "X";            // lands on the detached copy
//         }
//     }
//     o = createObject("component", "Fixture");
//     o.combo();
//     structKeyExists(o, "mk")
//         Lucee 5/6/7 / ACF / BoxLang -> true
//         RustCFML 0.108.0            -> false  (write discarded on return)
//
// The trigger is EXACTLY the member-call shape `this.method()`:
//   - a bare call `noop()`                     does NOT detach
//   - a bracket call `this["noop"]()`          does NOT detach
//   - a dot-READ `f = this.noop` (no call)     does NOT detach
//   - a dot-call on ANOTHER object             does NOT detach
//     (`other.noop()`, and even `this.member.noop()`)
//   - `variables`-scope writes always survive
//   - mutations of a NESTED struct reached through `this` escape even
//     while detached (the copy is shallow; struct refs are shared) --
//     pinned below as documented behaviour so a fix can't silently
//     regress reference semantics for nested containers.
//
// The failure is JIT-independent (identical under RUSTCFML_JIT=0 and
// RUSTCFML_JIT_THRESHOLD=1) and depth-independent (pure recursion to
// depth 200 without a dot-call stays green). The detach does not
// propagate upward: caller writes after a poisoned callee returns are
// fine, and a fresh method call on the same object sees true state.
//
// Why this matters for Wheels: the model save chain crosses TWO such
// dot-calls. invokeWithTransaction() calls this.$hashedConnectionArgs()
// (vendor/wheels/model/transactions.cfc) before dispatching $save -- so
// the primary key, timestamps, and persisted-state flags written below
// it all land on a detached copy: after update() the caller's object
// keeps a stale updatedAt and reports hasChanged() = true even though
// the DB row is correct. And $create() calls this.columnDataForProperty()
// (vendor/wheels/model/create.cfc) right before assigning the generated
// primary key -- the PK write dies one return later, so create() hands
// back an object with no id. Flipping each call site from `this.$x()`
// to the bare `$x()` restores full Lucee semantics, which is how the
// two sites were isolated.
// ============================================================

tdcd_obj = createObject("component", "ThisDotCallDetachFixture");
tdcd_obj.helperObj = createObject("component", "ThisDotCallDetachFixture");

// ------------------------------------------------------------
// RED row 1: dot-call, then NEW-KEY this-write in the same frame. The
// in-frame assert passes on every engine (the write IS visible inside
// the method); the after-return assert is the gap.
// ------------------------------------------------------------
tdcd_r = tdcd_obj.dotCallThenNewKeyWrite();
assert("dot-call then new-key this-write: visible in-frame (both engines)",
    tdcd_r, "inFrame=true");
assertTrue("dot-call then new-key this-write: this.mkNew SURVIVES the return",
    structKeyExists(tdcd_obj, "mkNew"));

// ------------------------------------------------------------
// RED row 2: dot-call, then EXISTING-KEY overwrite. On RustCFML the key
// silently REVERTS to its pre-call value when the frame returns.
// ------------------------------------------------------------
tdcd_r = tdcd_obj.dotCallThenOverwrite();
assert("dot-call then existing-key overwrite: visible in-frame (both engines)",
    tdcd_r, "inFrame=B");
assert("dot-call then existing-key overwrite: this.pre SURVIVES the return",
    tdcd_obj.pre, "B");

// ------------------------------------------------------------
// RED row 3: the dot-call poisons frames called AFTER it -- a this-write
// made inside a nested BARE call is also discarded on the outer return.
// ------------------------------------------------------------
tdcd_r = tdcd_obj.dotCallThenNestedBareWrite();
assert("dot-call then nested bare-call this-write: visible in-frame (both engines)",
    tdcd_r, "inFrame=true");
assertTrue("dot-call then nested bare-call this-write: this.nestedMark SURVIVES the return",
    structKeyExists(tdcd_obj, "nestedMark"));

// ------------------------------------------------------------
// GREEN controls -- the sharp wedge for the fix. Each is the closest
// NON-detaching neighbour of the trigger shape and already passes on
// RustCFML 0.108.0; a fix that breaks any of these overshot.
// ------------------------------------------------------------
tdcd_r = tdcd_obj.bareCallThenWrite();
assertTrue("CONTROL: bare call noop() does not detach (this.mkBare survives)",
    structKeyExists(tdcd_obj, "mkBare"));

tdcd_r = tdcd_obj.bracketCallThenWrite();
assertTrue("CONTROL: bracket call this['noop']() does not detach (this.mkBracket survives)",
    structKeyExists(tdcd_obj, "mkBracket"));

tdcd_r = tdcd_obj.dotReadThenWrite();
assertTrue("CONTROL: dot-READ f = this.noop does not detach (this.mkRead survives)",
    structKeyExists(tdcd_obj, "mkRead"));

tdcd_r = tdcd_obj.otherObjectDotCallThenWrite();
assertTrue("CONTROL: dot-call on a DIFFERENT object does not detach (this.mkOther survives)",
    structKeyExists(tdcd_obj, "mkOther"));

tdcd_r = tdcd_obj.helperMemberDotCallThenWrite();
assertTrue("CONTROL: dot-call through this.helperObj (receiver is not this) does not detach",
    structKeyExists(tdcd_obj, "mkHelper"));

tdcd_r = tdcd_obj.dotCallThenVariablesWrite();
assert("CONTROL: variables-scope write after a dot-call survives",
    tdcd_obj.getVMark(), "V");

// ------------------------------------------------------------
// PIN (documented behaviour, passes on BOTH engines): the detached copy
// is SHALLOW -- mutating a nested struct reached through `this` escapes
// the detach because the struct reference is shared. Pinned so an engine
// fix keeps reference semantics for nested containers.
// ------------------------------------------------------------
tdcd_r = tdcd_obj.dotCallThenNestedStructMutation();
assert("PIN: nested-struct mutation through this escapes (shared reference)",
    tdcd_obj.bag.n, 7);

suiteEnd();
</cfscript>
