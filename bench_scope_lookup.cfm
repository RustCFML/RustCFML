<cfscript>
// Scope lookup benchmark - exercises bare-name resolution through locals/arguments/variables
// Targets the lookup_name_in_scopes hot path

function testLocalLookup( n ) {
    var sum = 0;
    var a = 1;
    var b = 2;
    var c = 3;
    var d = 4;
    var e = 5;
    
    for ( var i = 1; i <= n; i++ ) {
        sum = sum + a + b + c + d + e;
    }
    return sum;
}

function testScopedLookup( n ) {
    var sum = 0;
    local.a = 1;
    local.b = 2;
    local.c = 3;
    local.d = 4;
    local.e = 5;
    
    for ( var i = 1; i <= n; i++ ) {
        sum = sum + local.a + local.b + local.c + local.d + local.e;
    }
    return sum;
}

function testMixedLookup( n ) {
    var sum = 0;
    var x = 1;
    local.y = 2;
    
    for ( var i = 1; i <= n; i++ ) {
        sum = sum + x + local.y;
        x = x + 1;
    }
    return sum;
}

iterations = 5000000;

// Warm-up
testLocalLookup( 10000 );
testScopedLookup( 10000 );
testMixedLookup( 10000 );

// Test 1: Local variable lookups (should hit slots)
start = getTickCount();
result1 = testLocalLookup( iterations );
elapsed1 = getTickCount() - start;
writeOutput( "LOCAL_LOOKUP " & elapsed1 & chr(10) );

// Test 2: Scoped lookups (local.x)
start = getTickCount();
result2 = testScopedLookup( iterations );
elapsed2 = getTickCount() - start;
writeOutput( "SCOPED_LOOKUP " & elapsed2 & chr(10) );

// Test 3: Mixed lookups
start = getTickCount();
result3 = testMixedLookup( iterations );
elapsed3 = getTickCount() - start;
writeOutput( "MIXED_LOOKUP " & elapsed3 & chr(10) );

writeOutput( "CHECK " & result1 & " " & result2 & " " & result3 & chr(10) );
</cfscript>
