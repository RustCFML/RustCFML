<cfscript>
suiteBegin("nested dotted stores (local/variables/arguments/bare roots) write in place");
// Expected values verified on Lucee 7.1.0.204.

function vivify() { local.r = {}; local.r.a.b.c = 1; return serializeJSON(local.r); }
assert("missing intermediates auto-vivify under local", vivify(), '{"a":{"b":{"c":1}}}');

// KNOWN DIVERGENCE (pre-existing, both the generic and in-place paths): a
// scalar intermediate. Lucee throws "Can't assign value to an Object of this
// type [String] with key [B]"; RustCFML replaces it with a struct.
function replaceScalar() { local.r = { a = "text" }; local.r.a.b = 1; return local.r.a.b; }
if (isRustCFML()) {
	assert("a scalar intermediate is replaced (RustCFML; Lucee throws)", replaceScalar(), 1);
} else {
	assertThrows("a scalar intermediate throws on Lucee", function() { replaceScalar(); });
}

// NOT asserted — known pre-existing divergence, tracked separately:
// `local.p.y = v` where `p` is a parameter creates a separate LOCAL `p` on Lucee
// (arguments.p untouched); RustCFML writes into the argument's struct.

function lazyArg(struct a) { arguments.a.k = 1; return lcase(listSort(structKeyList(a), "textnocase")); }
assert("arguments.<param>.k writes into the passed struct", lazyArg({z=0}), "k,z");

function eagerArg(struct a) { var n = arguments.len(); arguments.a.k = 1; return n & ":" & lcase(listSort(structKeyList(a), "textnocase")); }
assert("same on a frame with an eager arguments scope", eagerArg({z=0}), "1:k,z");

function byRef() { var src = { inner = {} }; var alias = src; alias.inner.k = "v"; return src.inner.k; }
assert("a nested store through an alias is visible through the original", byRef(), "v");

function chained() { local.r = {}; var x = local.r.a.b = 5; return x & "/" & local.r.a.b; }
assert("chained assignment leaves the value", chained(), "5/5");

// NOT asserted — known pre-existing divergence, tracked separately:
// `local.r.a.b = javaCast("null","")` keeps key `b` (null value) on Lucee;
// RustCFML deletes it.

function loopAccumulate() { local.acc = {}; for (var i = 1; i <= 3; i++) { local.acc.inner["k" & i] = i; local.acc.last.v = i; } return lcase(listSort(structKeyList(local.acc.inner), "textnocase")) & "|" & local.acc.last.v; }
assert("nested stores accumulate across loop iterations", loopAccumulate(), "k1,k2,k3|3");

variables.nps = {};
function varRoot() { variables.nps.a.b = "deep"; return variables.nps.a.b; }
assert("variables.x.y.z from a function", varRoot(), "deep");

pageRoot = {};
pageRoot.a.b = "page";
assert("bare root at page level", pageRoot.a.b, "page");
variables.pageVar.x.y = 1;
assert("variables root at page level", variables.pageVar.x.y, 1);
suiteEnd();
</cfscript>
