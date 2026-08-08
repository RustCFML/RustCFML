<cfscript>
suiteBegin("Closures see the LIVE variables scope (GitHub 316)");

// A closure must read the declaring `variables` scope as it is AT CALL TIME,
// not as it was when the closure was created (Lucee semantics — verified on
// Lucee 7). An UNSCOPED write (`counter = 2`) already propagated into the
// shared closure environment; the identical SCOPED write (`variables.counter =
// 2`) round-trips the whole scope struct through a different store path that
// skipped that sync, so every closure created earlier kept reading the value
// captured at creation time.

// --- 1. Reassignment through the scoped form is visible.
variables.g316counter = 1;
g316readScoped = function() { return variables.g316counter; };
g316readBare   = function() { return g316counter; };
variables.g316counter = 2;
assert("scoped read sees scoped reassignment", g316readScoped(), 2);
assert("unscoped read sees scoped reassignment", g316readBare(), 2);

// --- 2. The unscoped write form must keep working (no regression).
variables.g316other = 1;
g316readOther = function() { return variables.g316other; };
g316other = 3;
assert("scoped read sees unscoped reassignment", g316readOther(), 3);

// --- 3. A name assigned only AFTER the closure was created resolves.
g316late = function() { return variables.g316lateVar ?: "UNDEFINED"; };
variables.g316lateVar = "hello";
assert("closure sees a name assigned after its creation", g316late(), "hello");

// --- 4. Named page functions capture the same way closures do.
variables.g316named = 1;
function g316NamedReader() { return variables.g316named; }
variables.g316named = 9;
assert("named function sees the live scope too", g316NamedReader(), 9);

// --- 5. Arrow functions.
variables.g316arrowVal = 1;
g316arrow = () => variables.g316arrowVal;
variables.g316arrowVal = 4;
assert("arrow function sees the live scope", g316arrow(), 4);

// --- 6. The self-referencing-closure idiom: the closure is bound to the very
//     name its body calls, so at creation time that name does not exist yet.
//     This is what scheduling/retry code is written with, and it failed with
//     "Variable 'g316tick' is undefined".
variables.g316tick = function( n ) {
    return n <= 0 ? "done" : variables.g316tick( n - 1 );
};
assert("scoped self-referencing closure recurses", variables.g316tick( 3 ), "done");

// Mixed form: assigned scoped, calls itself unscoped.
variables.g316tock = function( n ) {
    return n <= 0 ? "done" : g316tock( n - 1 );
};
assert("scoped self-referencing closure, unscoped self-call", variables.g316tock( 3 ), "done");

// Unscoped assignment form (was already working — guard against regressing it).
g316tuck = function( n ) {
    return n <= 0 ? "done" : g316tuck( n - 1 );
};
assert("unscoped self-referencing closure recurses", g316tuck( 3 ), "done");

// --- 7. Writes from inside a closure still propagate outward.
variables.g316w = 0;
g316setter = function() { variables.g316w = 99; };
g316setter();
assert("closure write-back reaches the declaring scope", variables.g316w, 99);

// --- 8. Sibling closures share one live view of the scope.
variables.g316shared = "first";
g316readerA = function() { return variables.g316shared; };
g316readerB = function() { return variables.g316shared; };
variables.g316shared = "second";
assert("sibling closure A sees the update", g316readerA(), "second");
assert("sibling closure B sees the update", g316readerB(), "second");

// --- 9. Inside a function frame, a `var`-declared local must NOT be affected
//     by a same-named page-scope write (the env sync only touches keys the env
//     already holds, so it must not leak scope writes into locals).
function g316LocalShadow() {
    var g316shared = "local";
    var peek = function() { return g316shared; };
    variables.g316shared = "changed-again";
    return peek();
}
assert("a var-declared local is not clobbered by a scope write", g316LocalShadow(), "local");

suiteEnd();
</cfscript>
