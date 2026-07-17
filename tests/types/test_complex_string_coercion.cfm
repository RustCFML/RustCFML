<cfscript>
suiteBegin("Complex value → string coercion (Lucee parity) + shared-graph memoization");

// ---------------------------------------------------------------------------
// Lucee parity: coercing a COMPLEX value to a string (the `&` operator,
// multi-part interpolation, output, toString()) throws a catchable
// `expression`-typed error rather than dumping a `{k: v}` representation.
//
// The dump was not merely wrong-vs-Lucee: on a densely shared object graph
// (WireBox's injector↔binder↔builder) it expanded to an O(2^depth) string and
// hung ColdBox boot at ~14 GB RSS. Matching Lucee both fixes that and surfaces
// the real coercion site. Lucee reference messages (Lucee 7):
//   "" & {}          -> Can't cast Complex Object Type [Struct] to String
//   "" & []          -> Can't cast Complex Object Type [Array] to String
//   "" & queryNew()  -> Can't cast Complex Object Type [Query] to String
//   "" & new Foo()   -> Can't cast Component [Foo] to String
// ---------------------------------------------------------------------------

function throwsCoercion(fn) {
    try { fn(); return false; }
    catch (expression e) { return findNoCase("can't cast", e.message) > 0; }
    catch (any e) { return false; } // wrong type => fail the assertion
}

// --- the `&` concat operator throws for each complex type ---
assertTrue("struct concat throws expression", throwsCoercion(function(){ return "" & {a:1}; }));
assertTrue("array concat throws expression",  throwsCoercion(function(){ return "" & [1,2]; }));
assertTrue("query concat throws expression",  throwsCoercion(function(){ return "" & queryNew("id"); }));
assertTrue("closure concat throws expression", throwsCoercion(function(){ return "" & function(){}; }));

// --- multi-part interpolation ("a#x#b") is concat under the hood ---
assertTrue("multi-part interp of struct throws", throwsCoercion(function(){ var s = {a:1}; return "pre #s# post"; }));

// --- toString() BIF and writeOutput() also throw ---
assertTrue("toString(struct) throws", throwsCoercion(function(){ return toString({a:1}); }));
assertTrue("toString(array) throws",  throwsCoercion(function(){ return toString([1,2]); }));
assertTrue("writeOutput(array) throws", throwsCoercion(function(){ writeOutput([1,2]); }));

// --- the error is catchable specifically as `expression` (cross-frame) ---
caughtType = "";
try { x = "" & {a:1}; }
catch (expression e) { caughtType = "expression"; }
catch (any e) { caughtType = "any:" & e.type; }
assert("coercion error catchable as expression across frames", caughtType, "expression");

// ---------------------------------------------------------------------------
// SCALARS still coerce normally (Lucee casts these) — no regression.
// ---------------------------------------------------------------------------
assert("int concat",     "x" & 42,            "x42");
assert("double concat",  "v" & 1.5,           "v1.5");
assert("bool concat",    "b" & true,          "btrue");
assert("string concat",  "a" & "b",           "ab");
assertTrue("date concat coerces to a non-empty string", len("" & now()) > 0);

// A QueryColumn proxy coerces to its scalar (first/current row), never throws.
q = queryNew("name", "varchar", [ ["Ann"], ["Bo"] ]);
assert("query-column proxy concat uses first row", "n=" & q.name, "n=Ann");

// ---------------------------------------------------------------------------
// Single-part interpolation ("#x#") preserves the native value/type — this is
// deliberate (Lucee/ACF/BoxLang parity) and must NOT be turned into a throw.
// ---------------------------------------------------------------------------
srcStruct = {a:1};
single = "#srcStruct#";
assertTrue("single-part interp preserves the struct value", isStruct(single));

// ---------------------------------------------------------------------------
// Java-object shims are the EXCEPTION to the complex-value throw. A Java object
// returned by createObject("java", …) is represented internally as a struct,
// but Lucee coerces Java objects to their toString() in string contexts rather
// than throwing (verified on Lucee 7). ColdBox's CacheBox does
// `replace( createObject("java","java.util.UUID").randomUUID(), "-", "" )` and
// relies on this. Regression for the ColdBox-boot java-shim coercion fix.
// ---------------------------------------------------------------------------
uuidObj = createObject("java", "java.util.UUID").randomUUID();
assertFalse("a java.util.UUID object is not a simple value (Lucee parity)", isSimpleValue(uuidObj));
// It coerces (does NOT throw) in concat, and matches its own toString().
uuidConcat = "";
try { uuidConcat = "id=" & uuidObj; } catch (any e) { uuidConcat = "THREW:" & e.message; }
assert("java UUID concatenates to its toString (no throw)", uuidConcat, "id=" & uuidObj.toString());
assertTrue("java UUID coerces to a 36-char uuid string", reFind("id=[0-9a-fA-F\-]{36}$", uuidConcat) > 0);
// replace() (which coerces via as_string) also works on the raw java object.
assertTrue("replace() coerces a java UUID (dashes stripped -> 32 hex)", len(replace(uuidObj, "-", "", "all")) == 32);
// StringBuilder coerces to its buffered contents.
sb = createObject("java", "java.lang.StringBuilder").init("hi");
sb.append(" there");
assert("java StringBuilder coerces to its buffer", "" & sb, "hi there");

// ---------------------------------------------------------------------------
// Fix 1 — shared-sub-graph memoization. A struct whose members all point at the
// SAME child is a DAG: the child is reachable by many paths. The dump path
// (member .toString(), which Lucee-style still renders) used to re-render the
// shared child once PER PATH — O(2^depth). Memoization renders each clean
// container once and reuses it, so this completes quickly and the shared child
// renders identically everywhere. (If memoization regressed to the old
// per-path walk, this suite would hang rather than fail.)
// ---------------------------------------------------------------------------
function countOccur(hay, needle) {
    return ( len(hay) - len(replace(hay, needle, "", "all")) ) / len(needle);
}
child = {}; for (i = 1; i <= 40; i++) { child["k#i#"] = i * 7; }
wide = {};  for (i = 1; i <= 40; i++) { wide["ref#i#"] = child; } // 40 refs to ONE child
dumped = wide.toString();
assertTrue("shared-DAG dump completes and is non-empty", len(dumped) > 0);
// Every reference rendered the same child content (k1's entry "k1: 7" appears
// once per ref) — memoization reuses the child's string, it doesn't drop refs.
assert("shared child rendered under every ref (memo consistency)",
       countOccur(dumped, "k1: 7"), 40);

// Deep shared DAG completes without exponential blow-up in TIME.
deep = { leaf: "x" };
for (i = 1; i <= 22; i++) { deep = { l: deep, r: deep }; }
t0 = getTickCount();
deepStr = deep.toString();
elapsed = getTickCount() - t0;
assertTrue("deep shared-DAG dump completes", len(deepStr) > 0);
assertTrue("deep shared-DAG dump is not exponential in time (#elapsed#ms)", elapsed < 5000);

suiteEnd();
</cfscript>
