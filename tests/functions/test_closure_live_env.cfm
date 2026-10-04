<cfscript>
// A closure's scope is a live reference to its defining function's locals
// (Lucee). RustCFML captured a COPY when the closure was created and patched it
// up per call by scanning the CALLER's locals, which only worked when the caller
// was the definer: a callback handed to another function could not see a `var`
// its definer assigned after creating it. These pin the live-reference contract
// in both directions, including writes interleaved with the definer's own.
suiteBegin("Closure scope is a live reference to the defining function");

function cleInvoke(f) { var secret = "invoker-local"; return f(); }
function cleInvokeWith(f, x) { return f(x); }

function cleLateVar() {
    var early = 1;
    var g = function() { return isDefined("late") ? late : "UNDEF"; };
    var late = "assigned-after";
    return { direct: g(), viaInvoker: cleInvoke(g) };
}
r = cleLateVar();
assert("late var visible when called directly", r.direct, "assigned-after");
assert("late var visible through another function", r.viaInvoker, "assigned-after");

function cleNoDynamicScope() {
    var g = function() { return isDefined("secret") ? secret : "UNDEF"; };
    return cleInvoke(g);
}
assert("the invoker's own locals stay invisible", cleNoDynamicScope(), "UNDEF");

function cleUpdatedVar() {
    var n = 1;
    var g = function() { return n; };
    n = 2;
    var a = cleInvoke(g);
    n = 3;
    return [a, g(), cleInvoke(g)];
}
assert("updates after capture are visible", serializeJSON(cleUpdatedVar()), "[2,3,3]");

function cleClosureWrites() {
    var total = 0;
    var add = function(x) { total += x; };
    for (var i = 1; i <= 10; i++) {
        add(i);          // the closure writes `total`
        var seen = total; // the definer reads it straight back
    }
    cleInvokeWith(add, 100); // a write through a callback
    return [seen, total];
}
assert("closure writes reach the definer, also via a callback", serializeJSON(cleClosureWrites()), "[55,155]");

function cleInterleaved() {
    var a = 0;
    var b = 0;
    var bumpB = function() { b++; };
    for (var i = 1; i <= 50; i++) {
        a++;        // the definer's own write
        bumpB();    // the closure's write
        a += 1;     // the definer again, after the call
    }
    return [a, b, i];
}
assert("definer and closure writes interleaved", serializeJSON(cleInterleaved()), "[100,50,51]");

function cleLateFunction() {
    var g = function(x) { return helper(x) & "!"; };
    var helper = function(x) { return "h" & x; };
    return cleInvokeWith(g, 1);
}
assert("a function-valued var assigned after capture is callable", cleLateFunction(), "h1!");

function cleRecursive() {
    var fact = function(n) { return n <= 1 ? 1 : n * fact(n - 1); };
    return cleInvokeWith(fact, 5);
}
assert("a recursive var-scoped closure still resolves itself", cleRecursive(), 120);

suiteEnd();
</cfscript>
