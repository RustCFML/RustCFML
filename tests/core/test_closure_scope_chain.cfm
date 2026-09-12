<cfscript>
suiteBegin( "A closure resolves captured names through a live scope chain" );
// Every expectation below was verified against Lucee 7.1 (probe in the
// v0.667.0 notes). A closure frame REFERENCES the env of the function that
// defined it — and that env's own parents — instead of copying every captured
// key into its frame per call: reads see the current value, a classic-mode
// write to a captured name updates the variable where it lives, and a write to
// a NEW name goes to the defining scope's `variables`.
// (a) unscoped write to a new name: not a closure local, not the function's local — variables.
function a1() { var f = function(){ newVarA = 11; return isDefined( "local.newVarA" ) ? "closureLocal" : "notClosureLocal"; }; var r = f(); return r & "|fnLocal=" & isDefined( "local.newVarA" ) & "|vars=" & isDefined( "variables.newVarA" ); }
assert( "new-name write from a closure lands in variables", a1(), "notClosureLocal|fnLocal=false|vars=true" );
// (b) write to a captured function local updates it.
function b1() { var n = 1; var f = function(){ n = 7; }; f(); return n; }
assert( "closure write to a captured var-local updates it", b1(), 7 );
// (c) deferred closure sees the value the variable had when the function returned.
function c1() { var n = 1; var f = function(){ return n; }; n = 2; return f; }
g = c1();
assert( "deferred closure reads the captured var's last value", g(), 2 );
// (d) a closure nested in a closure mutates the outer closure's var-local.
h = function(){ var i = 0; var bump = function(){ i++; }; bump(); bump(); return i; };
assert( "nested closure mutates the enclosing closure's var-local", h(), 2 );
hh = function(){ var i = 0; var bump = function(){ i = i + 1; }; bump(); bump(); bump(); return i; };
assert( "nested closure assignment to the enclosing closure's var-local", hh(), 3 );
// (e) a parameter shadows a same-named page variable; writing it is local.
pv = 100;
k = function( pv ) { pv = pv + 1; return pv; };
assert( "parameter write stays local", k( 1 ), 2 );
assert( "page variable of the same name untouched", pv, 100 );
// (f) lexical, not dynamic: the definer's local wins over the caller's.
function f1() { var who = "definer"; return function(){ return who; }; }
function f2( fn ) { var who = "caller"; return fn(); }
assert( "closure sees its definer's local, never the caller's", f2( f1() ), "definer" );
// (g) higher-order callback mutating the enclosing function's local.
function g1() { var total = 0; [ 1, 2, 3 ].each( function( x ){ total += x; } ); return total; }
assert( "each() callback mutates the enclosing function's local", g1(), 6 );
// (h) page closure: new-name write lands in the page scope.
p = function(){ newPageVar = 5; };
p();
assert( "page closure new-name write lands in page variables", isDefined( "variables.newPageVar" ), true );
// (i) new-name write from a closure inside a function: variables, not local.
function i1() { var f = function(){ zz = 3; }; f(); return isDefined( "zz" ) & "/" & isDefined( "local.zz" ) & "/" & isDefined( "variables.zz" ); }
assert( "new-name write from a function's closure: variables scope", i1(), "true/false/true" );
// Recursion through a var-scoped function expression (its self-reference is
// a captured name resolved through the chain).
function factorial() { var fact = function( n ){ if ( n <= 1 ) return 1; return n * fact( n - 1 ); }; return fact( 5 ); }
assert( "recursive var-scoped function expression", factorial(), 120 );
function accumulate() { var total = 0; var walk = function( n ){ total += n; if ( n > 1 ) walk( n - 1 ); return total; }; return walk( 4 ); }
assert( "recursion that mutates a captured local across levels", accumulate(), 10 );
// A closure defined inside a closure inside a UDF (three levels) resolves the
// grandparent's local and the page scope.
pageLevel = "page";
function threeLevels() { var lvl1 = "one"; var middle = function(){ var lvl2 = "two"; var leaf = function(){ return lvl1 & "/" & lvl2 & "/" & pageLevel; }; return leaf(); }; return middle(); }
assert( "three-level chain resolves every level", threeLevels(), "one/two/page" );
// isDefined sees a captured name.
function seesCaptured( a ) { var f = function(){ return isDefined( "a" ) ? "bare:" & a : "noBare"; }; return f(); }
assert( "isDefined on a captured argument inside a closure", seesCaptured( 4 ), "bare:4" );
// A captured struct is mutated in place through the chain.
function capturedStruct() { var s = { n: 0 }; var f = function(){ s.n++; s.tag = "t"; }; f(); f(); return s.n & s.tag; }
assert( "captured struct member writes mutate the shared struct", capturedStruct(), "2t" );
// Loop counters and compound assignment on a captured local.
function counterLoop() { var c = 0; var f = function(){ for ( var j = 1; j <= 3; j++ ) c += j; }; f(); return c; }
assert( "compound assignment to a captured local", counterLoop(), 6 );
suiteEnd();
</cfscript>
