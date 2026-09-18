<cfscript>
suiteBegin("Core: an out-of-range / uncastable subscript WRITE throws (GH ##428)");

// Regression through v0.685.3: `SetIndex` decoded the subscript with
// `parse::<i64>().unwrap_or(0)` and then did `if one_based >= 1 { ... }` with no
// else, so every miss fell off the end of the op in silence. A write that
// neither lands nor complains is the worst of the three failure modes: a 0-based
// loop ported from another language (`for (i=0; i<len; i++) a[i] = ...`) stores
// nothing into element 1 and reports success. The READ half already matched
// Lucee (§107, v0.673.0) — only the write vanished.
//
// Every message below is measured on Lucee 7.1.0.204. Lucee casts the subscript
// to a number and truncates toward zero, so `a[1.7]` is a real write to element
// 1, `a["2.7"]` writes element 2, and `a[-1.7]` reports position [-1].

// Asserts both that the write throws AND that it throws Lucee's wording — the
// silent no-op this fixes would pass a bare assertThrows-free check, and a
// wrong message is what CFML code branching on `e.message` actually sees.
function throwsWith( required string label, required callback, required string expected ) {
    try {
        callback();
        assert( arguments.label, "no exception", arguments.expected );
    } catch( any e ) {
        assert( arguments.label, e.message, arguments.expected );
    }
}

// ---- 1. an array position below 1 ----------------------------------------
throwsWith( "a[0]=9 throws", function(){ var a = [ 1 ]; a[ 0 ] = 9; },
            "can not set Element at position [0]" );
throwsWith( "a[-1]=9 throws", function(){ var a = [ 1 ]; a[ -1 ] = 9; },
            "can not set Element at position [-1]" );
throwsWith( "a['0']=9 throws (numeric string)", function(){ var a = [ 1 ]; a[ '0' ] = 9; },
            "can not set Element at position [0]" );
throwsWith( "a[0].x=1 throws on the nested path", function(){ var a = [ 1 ]; a[ 0 ].x = 1; },
            "can not set Element at position [0]" );
throwsWith( "a[false]=9 throws (boolean casts to 0)", function(){ var a = [ 1 ]; a[ false ] = 9; },
            "can not set Element at position [0]" );

// Truncation is toward zero, and the TRUNCATED position is what is reported.
throwsWith( "a[0.5]=9 truncates to 0 and throws", function(){ var a = [ 1 ]; a[ 0.5 ] = 9; },
            "can not set Element at position [0]" );
throwsWith( "a[-1.7]=9 reports the truncated position", function(){ var a = [ 1 ]; a[ -1.7 ] = 9; },
            "can not set Element at position [-1]" );

// The array must be untouched after the refusal.
function untouchedAfterThrow() {
    var a = [ 1, 2, 3 ];
    try { a[ 0 ] = 9; } catch( any e ) {}
    return arrayLen( a ) & ":" & a[ 1 ];
}
assert( "a[0]=9 leaves the array unchanged", untouchedAfterThrow(), "3:1" );

// ---- 2. a subscript that will not cast to a number ------------------------
throwsWith( "a['x']=9 is a cast error", function(){ var a = [ 1 ]; a[ 'x' ] = 9; },
            "cannot cast [x] string to a number value" );
throwsWith( "a['0x2']=9 is a cast error", function(){ var a = [ 1 ]; a[ '0x2' ] = 9; },
            "cannot cast [0x2] string to a number value" );
throwsWith( "a['']=9 is an empty-string cast error", function(){ var a = [ 1 ]; a[ '' ] = 9; },
            "can't cast empty string to a number value" );
throwsWith( "a[' ']=9 is an empty-string cast error", function(){ var a = [ 1 ]; a[ ' ' ] = 9; },
            "can't cast empty string to a number value" );
throwsWith( "a[[1]]=9 refuses a complex subscript", function(){ var a = [ 1 ]; a[ [ 1 ] ] = 9; },
            "Can't cast Complex Object Type [Array] to String" );
throwsWith( "a[{}]=9 refuses a complex subscript", function(){ var a = [ 1 ]; a[ {} ] = 9; },
            "Can't cast Complex Object Type [Struct] to String" );
throwsWith( "st[[1]]=9 refuses a complex struct key", function(){ var st = {}; st[ [ 1 ] ] = 9; },
            "Can't cast Complex Object Type [Array] to String" );

// ---- 3. writes that were DROPPED but are legal on Lucee -------------------
// These are the other half of the same bug: the old i64 parse failed on a
// decimal or exponent string, fell back to 0, and the write disappeared.
function decimalStringSubscript() { var a = [ 1 ]; a[ '2.7' ] = 9; return arrayLen( a ) & ":" & a[ 2 ]; }
assert( "a['2.7']=9 writes element 2", decimalStringSubscript(), "2:9" );

function exponentStringSubscript() { var a = [ 1 ]; a[ '1e1' ] = 9; return arrayLen( a ); }
assert( "a['1e1']=9 grows to 10", exponentStringSubscript(), 10 );

function paddedStringSubscript() { var a = [ 1 ]; a[ ' 2 ' ] = 9; return a[ 2 ]; }
assert( "a[' 2 ']=9 trims and writes element 2", paddedStringSubscript(), 9 );

function decimalSubscript() { var a = [ 1 ]; a[ 1.7 ] = 9; return arrayLen( a ) & ":" & a[ 1 ]; }
assert( "a[1.7]=9 truncates to element 1", decimalSubscript(), "1:9" );

function booleanSubscript() { var a = [ 1 ]; a[ true ] = 9; return a[ 1 ]; }
assert( "a[true]=9 writes element 1", booleanSubscript(), 9 );

// ---- 4. in-range writes are unchanged -------------------------------------
function stillGrows() {
    var a = [ 1 ]; a[ 2 ] = 2;
    var b = [ 1 ]; b[ 4 ] = 4;
    return arrayLen( a ) & ":" & arrayLen( b ) & ":" & isNull( b[ 2 ] );
}
assert( "in-range and past-the-end writes still work", stillGrows(), "2:4:true" );

// ---- 5. a subscript write into a value with no members --------------------
throwsWith( "s=1; s[2]=3 throws", function(){ var s = 1; s[ 2 ] = 3; },
            "Can't assign value to an Object of this type [Number] with key [2]" );
throwsWith( "b=true; b[1]=1 throws", function(){ var b = true; b[ 1 ] = 1; },
            "Can't assign value to an Object of this type [Boolean] with key [1]" );
throwsWith( "s='abc'; s[1]='x' throws", function(){ var s = "abc"; s[ 1 ] = "x"; },
            "Can't assign value to an Object of this type [String] with key [1]" );

// ---- 6. a query row outside 1..recordCount --------------------------------
// Growing the column instead (the old behaviour) desynced it from every other
// column, so the QUERY came out corrupt rather than the write merely going
// missing.
throwsWith( "q['a'][0]=v throws", function(){
                var q = queryNew( "a", "varchar", [ [ "x" ] ] ); q[ "a" ][ 0 ] = "z"; },
            "invalid row number [0]" );
throwsWith( "q['a'][3]=v past recordCount throws", function(){
                var q = queryNew( "a", "varchar", [ [ "x" ] ] ); q[ "a" ][ 3 ] = "z"; },
            "invalid row number [3]" );

function inRangeQueryCellStillWrites() {
    var q = queryNew( "a", "varchar", [ [ "x" ] ] );
    q[ "a" ][ 1 ] = "z";
    return q[ "a" ][ 1 ];
}
assert( "q['a'][1]=v still writes the cell", inRangeQueryCellStillWrites(), "z" );

// ---- 7. a binary is a native byte array and will not expand ---------------
function binaryByteWrite() {
    var b = toBinary( toBase64( "ab" ) );
    b[ 1 ] = 65;
    return toString( b );
}
assert( "bin[1]=65 stores the byte", binaryByteWrite(), "Ab" );

throwsWith( "bin[9]=65 will not expand a native array", function(){
                var b = toBinary( toBase64( "ab" ) ); b[ 9 ] = 65; },
            "Invalid index [9] for Native Array, can't expand Native Arrays" );
throwsWith( "bin[0]=1 will not expand a native array", function(){
                var b = toBinary( toBase64( "ab" ) ); b[ 0 ] = 1; },
            "Invalid index [0] for Native Array, can't expand Native Arrays" );

// ---- 8. the refusal is catchable in the frame that made the write ---------
// `SetIndex` returns a bare `Err`, routed through `route_call_error`; an error
// that skipped the enclosing `try` would abort the whole request instead.
function caughtInFrame() {
    var a = [ 1 ];
    try { a[ 0 ] = 9; return "not thrown"; }
    catch( expression e ) { return "caught"; }
}
assert( "the refusal is catchable as type 'expression'", caughtInFrame(), "caught" );

suiteEnd();
</cfscript>
