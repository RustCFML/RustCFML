<cfscript>
suiteBegin("Core: script `param` attribute order-independence");

// ============================================================
// Background
// ============================================================
// The cfscript `param` statement accepts its attributes in ANY order:
//   param name="x" default="y";
//   param default="y" name="x";     // <- default-first (Masa/Mura style)
//   param type="numeric" name="x" default=5;
// RustCFML's parser used to detect the named-attribute form ONLY when the FIRST
// token was `name`. So `param default="" name="x"` was misparsed as the
// shorthand form (`param default = ""`), leaving the `name="x"` fragment as a
// separate statement — the variable `x` was NEVER parametrized.
//
// Masa CMS's clogin.cfc before() defaults every request-context key with
// `param default="" name="arguments.rc.status";` (default-first), so `rc.status`
// stayed undefined and the login view threw "Variable 'status' is undefined".
// ============================================================

// --- simple variable, both orders ---
param default="dv" name="simpleA";
assert("default-first defines the variable", isDefined("simpleA"), true);
assert("default-first applies the default", simpleA, "dv");

param name="simpleB" default="dv";
assert("name-first still works", simpleB, "dv");

// --- default-first must NOT clobber an existing value ---
existing = "keep";
param default="other" name="existing";
assert("param does not overwrite an existing value", existing, "keep");

// --- deep struct path, default-first ---
st = structNew();
param default="dx" name="st.a";
assert("default-first creates a deep struct key", structKeyExists(st, "a"), true);
assert("default-first deep key gets the default", st.a, "dx");

// --- typed param, attributes reordered ---
param default=5 name="typedN" type="numeric";
assert("type/default/name in any order: value", typedN, 5);

// --- the exact Masa pattern: default-first on arguments.rc.KEY, by reference ---
function before(rc) {
    param default="" name="arguments.rc.status";
    param default="" name="arguments.rc.returnurl";
}
ctx = structNew();
before(ctx);
assert("default-first param on arguments.X writes through by reference (status)", structKeyExists(ctx, "status"), true);
assert("default-first param on arguments.X writes through by reference (returnurl)", structKeyExists(ctx, "returnurl"), true);
assert("by-reference defaulted value is empty string", ctx.status, "");

suiteEnd();
</cfscript>
