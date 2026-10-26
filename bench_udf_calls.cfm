<cfscript>
// Micro-benchmark for UDF call overhead
// Usage: cargo run --release -- bench_udf_calls.cfm

function noop() {
    return 1;
}

function noopWith1Arg(a) {
    return a;
}

function noopWith3Args(a, b, c) {
    return a + b + c;
}

function recursive(n) {
    if (n <= 0) return 0;
    return 1 + recursive(n - 1);
}

// Warm up JIT / caches
for (i = 1; i <= 1000; i++) {
    noop();
}

iterations = 1000000;

// Test 1: Simple no-arg function call
start = getTickCount();
for (i = 1; i <= iterations; i++) {
    noop();
}
elapsed1 = getTickCount() - start;
writeOutput("Test 1 (no-arg UDF): #iterations# calls in #elapsed1#ms = #numberFormat(iterations / elapsed1, '999,999')# calls/ms~n");

// Test 2: 1-arg function call
start = getTickCount();
for (i = 1; i <= iterations; i++) {
    noopWith1Arg(i);
}
elapsed2 = getTickCount() - start;
writeOutput("Test 2 (1-arg UDF): #iterations# calls in #elapsed2#ms = #numberFormat(iterations / elapsed2, '999,999')# calls/ms~n");

// Test 3: 3-arg function call
start = getTickCount();
for (i = 1; i <= iterations; i++) {
    noopWith3Args(i, i+1, i+2);
}
elapsed3 = getTickCount() - start;
writeOutput("Test 3 (3-arg UDF): #iterations# calls in #elapsed3#ms = #numberFormat(iterations / elapsed3, '999,999')# calls/ms~n");

// Test 4: Recursive call (depth 100)
iterations4 = 10000;
start = getTickCount();
for (i = 1; i <= iterations4; i++) {
    recursive(100);
}
elapsed4 = getTickCount() - start;
writeOutput("Test 4 (recursive 100-deep): #iterations4# calls in #elapsed4#ms = #numberFormat(iterations4 / elapsed4, '999,999')# calls/ms~n");

writeOutput("~nTotal time: #elapsed1 + elapsed2 + elapsed3 + elapsed4#ms~n");
</cfscript>
