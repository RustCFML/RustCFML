<cfscript>
// Lucee 7 Compatibility Tests: Closures and Higher-Order Functions
// Synthesized from https://github.com/lucee/Lucee/tree/7.0/test
// Original tests Copyright (c) 2014, the Railo Company LLC / Copyright (c) 2015-2016, Lucee Association Switzerland
// Licensed under the GNU Lesser General Public License v2.1
// Adapted for RustCFML test harness

// ============================================================
// Basic Closures
// ============================================================
suiteBegin("Lucee7: Basic Closures");
fn = function(x) { return x * 2; };
assert("basic closure", fn(5), 10);

fn = function(a, b) { return a + b; };
assert("two arg closure", fn(3, 4), 7);

fn = function() { return 42; };
assert("no arg closure", fn(), 42);

fn = function(x) { return x * x; };
assert("closure square", fn(7), 49);
suiteEnd();

// ============================================================
// Closure Scope Capture
// ============================================================
suiteBegin("Lucee7: Closure Scope Capture");
x = 10;
fn = function() { return x; };
assert("captures outer var", fn(), 10);

x = 10;
fn = function() { x = 20; };
fn();
assert("mutates captured var", x, 20);

counter = 0;
inc = function() { counter++; };
inc();
inc();
inc();
assert("captured mutation persists", counter, 3);

// Capture multiple variables
a = 1;
b = 2;
fn = function() { return a + b; };
assert("captures multiple vars", fn(), 3);
suiteEnd();

// ============================================================
// Closures as Arguments
// ============================================================
suiteBegin("Lucee7: Closures as Arguments");
arr = [3, 1, 2];
arraySort(arr, function(a, b) { return a - b; });
assert("closure sort asc", arrayToList(arr), "1,2,3");

result = arrayFilter([1, 2, 3, 4, 5], function(item) { return item <= 3; });
assert("closure filter lte 3", arrayToList(result), "1,2,3");

result = arrayMap([1, 2, 3], function(item) { return item * 10; });
assert("closure map", arrayToList(result), "10,20,30");

result = arrayFilter([1, 2, 3, 4, 5], function(item) { return item > 3; });
assert("closure filter", arrayToList(result), "4,5");

sum = arrayReduce([1, 2, 3, 4], function(acc, item) { return acc + item; }, 0);
assert("closure reduce", sum, 10);

result = arrayMap([10, 20, 30], function(item, index) { return index; });
assert("closure map with index", arrayToList(result), "1,2,3");
suiteEnd();

// ============================================================
// Closure Returning Closure (Factory Pattern)
// ============================================================
suiteBegin("Lucee7: Closure Factory");
makeAdder = function(x) {
    return function(y) { return x + y; };
};
add5 = makeAdder(5);
assert("closure factory add5(3)", add5(3), 8);
assert("closure factory add5(10)", add5(10), 15);

add10 = makeAdder(10);
assert("closure factory add10(3)", add10(3), 13);

// Ensure separate closures are independent
assert("factories are independent", add5(1) + add10(1), 17);

makeMultiplier = function(factor) {
    return function(x) { return x * factor; };
};
double = makeMultiplier(2);
triple = makeMultiplier(3);
assert("multiplier factory double", double(5), 10);
assert("multiplier factory triple", triple(5), 15);
suiteEnd();

// ============================================================
// Deferred nested closures capture a LIVE outer var (Lucee parity)
// Mirrors the TestBox describe/beforeEach/it shape that broke cfflow:
// closures are registered at increasing lexical depth, an outer variable
// is reassigned AFTER registration, then the deferred readers run. Lucee
// captures the variable (not a snapshot), so every depth sees the new
// value. RustCFML previously saw it only at the sibling level (depth 0)
// and read a stale copy at any deeper nesting.
suiteBegin("Lucee7: Deferred nested closure live capture");
deferredReaders = [];
function buildDeferred(){
    var v = "INITIAL";
    var mutate = function(){ v = "WRITTEN"; };
    deferredReaders.append( function(){ return v; } );                 // depth 0 (sibling)
    var d1 = function(){
        deferredReaders.append( function(){ return v; } );             // depth 1
        var d2 = function(){
            deferredReaders.append( function(){ return v; } );         // depth 2
            var d3 = function(){
                deferredReaders.append( function(){ return v; } );     // depth 3
            }; d3();
        }; d2();
    }; d1();
    mutate();   // reassign AFTER all readers were registered
}
buildDeferred();
// Calling an array element directly (`arr[1]()`) is a RustCFML superset —
// Lucee reads it as a member function named "1" and throws. The closures
// themselves are cross-engine; only this call spelling is not.
if ( isRustCFML() ) {
    assert("deferred closure depth 0 sees live value", deferredReaders[1](), "WRITTEN");
    assert("deferred closure depth 1 sees live value", deferredReaders[2](), "WRITTEN");
    assert("deferred closure depth 2 sees live value", deferredReaders[3](), "WRITTEN");
    assert("deferred closure depth 3 sees live value", deferredReaders[4](), "WRITTEN");
}

// A closure factory nested inside must STILL capture its own per-call arg,
// even though the shared outer scope holds a same-named variable. Guards
// against the live-capture chain leaking an ancestor value over a param.
x = 999; // decoy same-named page var
nestedFactory = function(x){ return function(){ return x; }; };
capA = nestedFactory(5);
capB = nestedFactory(10);
assert("nested factory keeps own arg A", capA(), 5);
assert("nested factory keeps own arg B", capB(), 10);
suiteEnd();

// ============================================================
// IIf (from Lucee IIf.cfc)
// ============================================================
suiteBegin("Lucee7: IIf");
assert("iif true", iif(true, de("yes"), de("no")), "yes");
assert("iif false", iif(false, de("yes"), de("no")), "no");
assert("iif expression", iif(1 == 1, de("equal"), de("not equal")), "equal");
assert("iif numeric condition", iif(1, de("truthy"), de("falsy")), "truthy");
assert("iif zero condition", iif(0, de("truthy"), de("falsy")), "falsy");
suiteEnd();

// ============================================================
// Named Function Expressions
// ============================================================
suiteBegin("Lucee7: Named Functions");
function add(a, b) { return a + b; }
assert("named function", add(2, 3), 5);

function multiply(a, b) { return a * b; }
assert("named multiply", multiply(4, 5), 20);

function factorial(n) {
    if (n <= 1) return 1;
    return n * factorial(n - 1);
}
assert("recursive function", factorial(5), 120);
suiteEnd();

// ============================================================
// Default Argument Values
// ============================================================
suiteBegin("Lucee7: Default Argument Values");
function greet(name = "World") { return "Hello " & name; }
assert("default arg used", greet(), "Hello World");
assert("provided arg used", greet("CFML"), "Hello CFML");

function calc(a, b = 10, op = "add") {
    if (op == "add") return a + b;
    if (op == "mul") return a * b;
    return 0;
}
assert("partial default args", calc(5), 15);
assert("override one default", calc(5, 20), 25);
assert("override all defaults", calc(5, 3, "mul"), 15);
suiteEnd();

// ============================================================
// Variable Arguments
// ============================================================
suiteBegin("Lucee7: Variable Arguments");
function countArgs() { return structCount(arguments); }
assert("var args count 3", countArgs(1, 2, 3), 3);
assert("var args count 0", countArgs(), 0);
assert("var args count 5", countArgs("a", "b", "c", "d", "e"), 5);

function getFirstTwoSum(a, b) {
    return arguments[1] + arguments[2];
}
assert("arguments positional sum", getFirstTwoSum(10, 20), 30);
assert("arguments named sum", getFirstTwoSum(a=3, b=7), 10);

function firstArg() {
    if (structCount(arguments) > 0) return arguments[1];
    return "none";
}
assert("arguments positional access", firstArg("hello"), "hello");
assert("arguments empty fallback", firstArg(), "none");
suiteEnd();
</cfscript>
