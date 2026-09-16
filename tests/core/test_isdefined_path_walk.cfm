<cfscript>
suiteBegin("Core: isDefined() dotted/bracket path walk (GH ##427)");

// Two defects, both measured against Lucee 5.4.8.2 and 7.1.0.204:
//
// 1. Regression v0.661–v0.685: a path rooted at the `arguments` scope was always
//    false past the parameter itself. `isDefined("arguments.s.k")` folds its
//    LITERAL into `BytecodeOp::IsDefined`, so the two static scanners that
//    decide whether a frame needs the eager `arguments` struct — which only
//    looked at `BytecodeOp::String` and `SetScopePath` — went blind to it. The
//    frame then skipped the struct and the probe had no scope to walk.
//    Preside's `FormsService._parseRules` gates on `isDefined(
//    "arguments.field.rule" )`, so every form field lost its validation rules.
//
// 2. A bracket subscript inside the path (`s.arr[1].q`) was false in EVERY
//    scope: the walk split on "." only, leaving `arr[1]` as a literal key.

_doc = xmlParse( '<field><rule validator="required"><param name="a" value="b"/></rule></field>' );
_x   = _doc.field;
_s   = { k = 1, arr = [ { q = 2 } ], nested = { deep = 3 } };

function viaArgs( required any s, required any x ) {
    return {
          k       = isDefined( "arguments.s.k" )
        , deep    = isDefined( "arguments.s.nested.deep" )
        , xmlKids = isDefined( "arguments.x.rule" )
        , indexed = isDefined( "arguments.s.arr[1].q" )
        , missing = isDefined( "arguments.s.nope" )
        , oob     = isDefined( "arguments.s.arr[2].q" )
    };
}
function viaBare( required any s, required any x ) {
    return { k = isDefined( "s.k" ), xmlKids = isDefined( "x.rule" ) };
}
function viaLocal( required any s ) {
    var l = arguments.s;
    return { k = isDefined( "local.l.k" ), indexed = isDefined( "local.l.arr[1].q" ), oob = isDefined( "local.l.arr[9].q" ) };
}

// 1. arguments-rooted paths
_a = viaArgs( _s, _x );
assert("arguments.s.k", _a.k, true);
assert("arguments.s.nested.deep", _a.deep, true);
assert("arguments.x.rule (XML children)", _a.xmlKids, true);
assert("arguments.s.arr[1].q", _a.indexed, true);
assert("arguments.s.nope is not defined", _a.missing, false);
assert("arguments.s.arr[2].q is out of range", _a.oob, false);

_b = viaBare( _s, _x );
assert("bare s.k inside the same function", _b.k, true);
assert("bare x.rule inside the same function", _b.xmlKids, true);

_l = viaLocal( _s );
assert("local.l.k", _l.k, true);
assert("local.l.arr[1].q", _l.indexed, true);
assert("local.l.arr[9].q is out of range", _l.oob, false);

// 2. bracket subscripts at page scope
assert("page: struct member", isDefined( "_s.k" ), true);
assert("page: variables-scoped struct member", isDefined( "variables._s.k" ), true);
assert("page: index into an array of structs", isDefined( "_s.arr[1].q" ), true);
assert("page: index into XML children", isDefined( "_x.rule[1].param" ), true);
assert("page: out-of-range index is not defined", isDefined( "_s.arr[2].q" ), false);
assert("page: zero index is not defined", isDefined( "_s.arr[0]" ), false);
assert("page: missing member under a resolved index", isDefined( "_s.arr[1].nope" ), false);

// A quoted subscript is a KEY, not an index (mirrors getVariable/structGet).
_k = { k = 1, "1" = "one" };
assert("quoted subscript reads a key", isDefined( "_k['k']" ), true);
assert("numeric subscript on a struct falls back to the key", isDefined( "_k[1]" ), true);
assert("numeric subscript on a struct with no such key", isDefined( "_k[2]" ), false);

suiteEnd();
</cfscript>
