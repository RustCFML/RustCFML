<cfscript>
suiteBegin("OOP: component method named like a builtin");

// ============================================================
// Background
// ============================================================
// On Lucee and Adobe ColdFusion a component may define a method whose name
// collides with a built-in function (e.g. canonicalize, reverse, len, find).
// Object method dispatch takes precedence, so obj.canonicalize(x) calls the
// COMPONENT METHOD, not the builtin. This is extremely common in frameworks --
// Moopa's route_url.cfc defines canonicalize() and the whole routing layer
// calls variables.routeUrl.canonicalize(url).
//
// RustCFML enforces a Lucee-parity rule that a top-level user function cannot
// redefine a builtin (correct: `function abs(){}` at script top level throws on
// Lucee). But that rule over-reaches to COMPONENT METHODS: the DefineFunction op
// rejects/drops any non-"__" function whose name is a builtin, so a CFC method
// named like a builtin never registers and obj.canonicalize() fails with
// "Component has no function with name [canonicalize]". Lucee allows it.
//
// WHAT TO FIX
// -----------
// crates/cfml-vm/src/lib.rs, the DefineFunction op handler. The builtin-collision
// guard ("The name [X] is already used by a built in Function") must apply ONLY
// to top-level function declarations, NOT to component methods. A method defined
// inside a CFC must register and be dispatchable via obj.name() even when its
// name matches a builtin. (Top-level redefinition should still throw -- see the
// existing tests/core/test_builtin_shadowing.cfm, which this does not change.)
// ============================================================

function callMethod(required string fixture, required string method, required string arg) {
    try {
        var o = createObject("component", arguments.fixture);
        if (!isObject(o)) {
            return "NOT-A-COMPONENT";
        }
        return invoke(o, arguments.method, { u = arguments.arg });
    } catch (any e) {
        return "THREW: " & e.message;
    }
}

// --- control: ordinary method name already works ---
assert("control: ordinary method dispatches",
    callMethod("ControlMethodFixture", "ping", "x"), "ping:x");

// --- gap: methods whose names collide with builtins must still dispatch ---
assert("component method named 'canonicalize' (a builtin) dispatches",
    callMethod("BuiltinNameMethodFixture", "canonicalize", "x"), "canon:x");

assert("component method named 'reverse' (a builtin) dispatches",
    callMethod("BuiltinNameMethodFixture", "reverse", "x"), "rev:x");

// a builtin-named method must not poison sibling methods on the same component
assert("ordinary sibling method on the same component still dispatches",
    callMethod("BuiltinNameMethodFixture", "plain", "x"), "plain:x");

suiteEnd();
</cfscript>
