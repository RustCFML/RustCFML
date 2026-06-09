<cfscript>
suiteBegin("Core: typed return type on a top-level cfscript function");

// On Lucee 5/6/7, Adobe ColdFusion, and BoxLang, a top-level (non-component)
// cfscript function may carry a return-type annotation -- `struct function f()`
// -- exactly like a component method. RustCFML 0.92.0 misparses the leading
// type token at the top level as a bare expression STATEMENT, so
// `struct function f(){...}` throws "Variable 'struct' is undefined" at runtime.
//
// The same declaration INSIDE a component works, and an UNTYPED top-level
// function works -- both are exercised here as controls so the test can't pass
// for the wrong reason. Every type keyword regresses identically
// (struct/array/string/numeric/boolean/query/any/void).
//
// Surfaced booting the Wheels framework: vendor/wheels/public/helpers.cfm:293
// declares `struct function $returnInternalDocumentation(...)`, and many view
// helper .cfm files declare typed top-level functions.
//
// On RustCFML this surfaces as a runtime ERROR at the first typed declaration
// (so tests/runner.cfm reports `ERROR | ... | Variable 'struct' is undefined`),
// not as assertion failures.

// --- CONTROL: untyped top-level fn works on BOTH engines (wiring guard) ---
// (A typed return type INSIDE a component also works on RustCFML 0.92.0 -- the
// regression is specific to TOP-LEVEL function declarations.)
function plainTop() { return {a: 1}; }
assertTrue("control: untyped top-level fn returns a struct", isStruct(plainTop()));

// --- GAP: typed return types on TOP-LEVEL functions ---
struct  function makeStruct() { return {a: 1}; }
array   function makeArray()  { return [1, 2, 3]; }
string  function makeString() { return "hi"; }
assertTrue("struct-typed top-level fn returns a struct", isStruct(makeStruct()));
assertTrue("array-typed top-level fn returns an array", isArray(makeArray()));
assert("string-typed top-level fn returns its value", makeString(), "hi");

suiteEnd();
</cfscript>
