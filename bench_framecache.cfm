<cfscript>
// Benchmark that exercises variables scope (FrameScopeCache path)

// Test 1: Variables scope lookups
function testVariablesLookup( n ) {
    variables.x = 1;
    variables.y = 2;
    variables.z = 3;
    var sum = 0;
    for ( var i = 1; i <= n; i++ ) {
        sum += variables.x + variables.y + variables.z;
        variables.x = variables.x + 0; // Force cache refresh
    }
    return sum;
}

// Test 2: argumentCollection
function testArgumentCollection( n ) {
    var sum = 0;
    for ( var i = 1; i <= n; i++ ) {
        sum += helper( argumentCollection={a:1, b:2, c:3} );
    }
    return sum;
}

function helper( required numeric a, required numeric b, required numeric c ) {
    return a + b + c;
}

iterations = 2000000;

// Warm up
testVariablesLookup( 10000 );
testArgumentCollection( 10000 );

// Test 1: Variables scope lookups (exercises FrameScopeCache)
start = getTickCount();
result1 = testVariablesLookup( iterations );
elapsed1 = getTickCount() - start;
writeOutput( "VARIABLES_LOOKUP " & elapsed1 & chr(10) );

// Test 2: argumentCollection (exercises reorder_named_args)
start = getTickCount();
result2 = testArgumentCollection( iterations );
elapsed2 = getTickCount() - start;
writeOutput( "ARG_COLLECTION " & elapsed2 & chr(10) );

writeOutput( "CHECK " & result1 & " " & result2 & chr(10) );
</cfscript>
