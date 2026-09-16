<cfscript>
suiteBegin("Core: nested assignment auto-creates a missing index/key (GH ##426)");

// Regression v0.661-v0.685: an assignment whose LHS traverses a not-yet-existing
// array index or struct key threw the READ error instead of auto-creating the
// container. §107 (v0.673.0) made a bracket read of a missing key / out-of-range
// index throw with Lucee's wording, and the EXPRESSION-position assignment path
// compiled its base with the strict expression compiler — unlike its
// `Statement::Assignment` twin, which already used the auto-vivifying
// `compile_index_assign_base`. A cfscript `a[2].x = 1;` takes the expression
// path, so the whole assignment died on the read.
//
// Lucee 5.4.8.2 and 7.1.0.204 both auto-extend/auto-create here; only a plain
// READ of the missing index throws. Every expectation below is measured on
// Lucee 7.1.0.204.

function nestedIndexOnShortArray() {
    var a = [ {} ];
    a[ 2 ].x = 1;
    return { len = arrayLen( a ), val = a[ 2 ].x };
}
_r = nestedIndexOnShortArray();
assert("a[2].x=1 extends the array", _r.len, 2);
assert("a[2].x=1 stores the value", _r.val, 1);

function nestedIndexLeavesAGap() {
    var a = [ {} ];
    a[ 3 ].x = 1;
    return { len = arrayLen( a ), val = a[ 3 ].x, gapNull = isNull( a[ 2 ] ) };
}
_r = nestedIndexLeavesAGap();
assert("a[3].x=1 extends to 3", _r.len, 3);
assert("a[3].x=1 stores the value", _r.val, 1);
assert("the skipped element is null", _r.gapNull, true);

function missingKeyOnThePath() {
    var a = [ {} ];
    a[ 1 ].y.z = 1;          // `y` does not exist yet
    return a[ 1 ].y.z;
}
assert("a[1].y.z=1 creates the intermediate struct", missingKeyOnThePath(), 1);

// Deeper / mixed shapes, same rule.
function mixedShapes() {
    var s = {};        s[ "a" ].b   = 1;
    var t = {};        t.a[ 1 ].b   = 2;
    var u = [ {} ];    u[ 2 ].x.y   = 3;
    var v = [ {} ];    v[ 2 ][ 1 ].x = 4;
    var w = { a = [ {} ] }; w.a[ 2 ].b = 5;
    return { s = s.a.b, t = t.a[ 1 ].b, u = u[ 2 ].x.y, v = v[ 2 ][ 1 ].x, w = w.a[ 2 ].b, wlen = arrayLen( w.a ) };
}
_m = mixedShapes();
assert("s['a'].b=1 creates the key", _m.s, 1);
assert("t.a[1].b=2 creates the array", _m.t, 2);
assert("u[2].x.y=3 creates index and key", _m.u, 3);
assert("v[2][1].x=4 creates nested arrays", _m.v, 4);
assert("w.a[2].b=5 extends a nested array", _m.w, 5);
assert("w.a[2].b=5 leaves length 2", _m.wlen, 2);

// In VALUE position the inner assignment must still yield the value.
function inValuePosition() {
    var a = [ {} ];
    var got = ( a[ 2 ].x = 7 );
    return { got = got, stored = a[ 2 ].x };
}
_v = inValuePosition();
assert("(a[2].x=7) evaluates to the value", _v.got, 7);
assert("(a[2].x=7) also stores it", _v.stored, 7);

// The building-rows idiom from the issue (Preside WebsiteUserActionServiceTest).
function buildRows() {
    var rows = [ {} ];
    for( var i = 1; i <= 3; i++ ) {
        rows[ i ].filter = "f" & i;
    }
    return { len = arrayLen( rows ), last = rows[ 3 ].filter };
}
_b = buildRows();
assert("row-building loop extends the array", _b.len, 3);
assert("row-building loop writes each row", _b.last, "f3");

// Plain index assignment was already correct — keep it that way.
function plainIndexAssign() {
    var a = [ 1 ];  a[ 2 ] = 2;
    var b = [ 1 ];  b[ 4 ] = 4;
    return { a = arrayLen( a ), b = arrayLen( b ), gap = isNull( b[ 2 ] ) };
}
_p = plainIndexAssign();
assert("a[2]=2 extends by one", _p.a, 2);
assert("b[4]=4 extends with a gap", _p.b, 4);
assert("the gap element is null", _p.gap, true);

// A READ of a missing index/key still throws (§107) — the fix must not make the
// read lenient again.
assertThrows("reading an out-of-range index still throws", function(){ var a = [ 1 ]; return a[ 2 ]; });
assertThrows("reading a missing struct key still throws", function(){ var s = {}; return s.a.b; });

suiteEnd();
</cfscript>
