<cfscript>
suiteBegin("Core: local.X write must not shadow arguments.X reads in the same frame");

// ============================================================
// Background
// ============================================================
// `local` and `arguments` are SEPARATE scopes within one call frame. Writing
// `local.X` (or `var X`) creates/updates a slot in the LOCAL scope only; an
// explicit `arguments.X` read afterwards must keep resolving to the argument
// value — the passed value, or the declared default. RustCFML 0.108.0
// collapses the two views after the local write: every subsequent
// `arguments.X` read returns the local value instead.
//
//   function f(string params = "") {
//       local.params = {controller: "x"};
//       return Len(arguments.params);
//   }
//   f()                  RustCFML 0.108.0 -> 1 (the struct!)   Lucee -> 0 ("" default)
//   f(params = "page=2") RustCFML 0.108.0 -> the struct        Lucee -> "page=2"
//
// Bare reads are NOT part of the gap: an unscoped `X` resolves through the
// scope cascade (local before arguments) on every engine, so after the local
// write a bare `params` IS the struct on Lucee too. The controls below pin
// that, so a fix cannot overcorrect bare-name resolution. Reads BEFORE the
// local write, distinct names, and explicit arguments-scope writes are also
// already correct — only the read-after-local-write path aliases.
//
// Scoped-resolution sibling of the local-scope family: #77 (callee local
// declaration clobbered the caller's; fixed v0.92.0) and #93 (caller's local
// visible to an undeclared callee's absence-checks; open) covered local-vs-
// local across frames — this one conflates the local and arguments views of a
// SINGLE frame.
// ============================================================

// --- (1) THE GAP: defaulted, unpassed argument survives a same-name local write ---
function argShadowDefaulted(string params = "") {
	local.params = {controller: "x"};
	return Len(arguments.params) & "|" & IsSimpleValue(arguments.params);
}
assert("unpassed default survives a local.X write (Len 0, still simple)",
	argShadowDefaulted(), "0|true");

// --- (2) THE GAP: a PASSED value also survives ---
function argShadowPassed(string params = "") {
	local.params = {controller: "x"};
	return toString(arguments.params);
}
assert("passed value survives a local.X write",
	argShadowPassed(params = "page=2"), "page=2");

// --- (3) THE GAP: the `var X` form (identical semantics to local.X) ---
function argShadowVarForm(string params = "") {
	var params = {controller: "x"};
	return Len(arguments.params);
}
assert("a `var X` write does not shadow arguments.X either",
	argShadowVarForm(), 0);

// --- (4) THE GAP: read-before-write is already correct; read-after must match ---
function argShadowPrePost(string params = "") {
	var pre = Len(arguments.params);
	local.params = {controller: "x"};
	return pre & "|" & Len(arguments.params);
}
assert("arguments.X reads identical before AND after the local write",
	argShadowPrePost(), "0|0");

// --- (5) CONTROL (green on both engines): bare X follows the scope cascade —
//     local wins. Pins the boundary so a fix can't break bare-name reads. ---
function argShadowBareRead(string params = "") {
	local.params = {controller: "x"};
	return IsStruct(params);
}
assertTrue("CONTROL: bare read resolves to the LOCAL struct (scope cascade)",
	argShadowBareRead());

// --- (6) CONTROL: a different local name leaves arguments.X alone ---
function argShadowDistinct(string params = "") {
	local.other = {controller: "x"};
	return Len(arguments.params);
}
assert("CONTROL: a distinct local name leaves arguments.X alone",
	argShadowDistinct(), 0);

// --- (7) CONTROL: an explicit arguments-scope write lands in arguments ---
function argShadowArgWrite(string params = "") {
	arguments.params = "a=1";
	return arguments.params;
}
assert("CONTROL: explicit arguments.X write then read round-trips",
	argShadowArgWrite(), "a=1");

// --- (8) CONTROL: the two slots stay separate in BOTH directions ---
function argShadowBothWrites(string params = "") {
	local.params = {controller: "x"};
	arguments.params = "ARGWRITE";
	return IsStruct(local.params) & "|" & toString(arguments.params);
}
assert("CONTROL: arguments.X write does not clobber local.X",
	argShadowBothWrites(), "true|ARGWRITE");

// --- (9/10) THE GAP, component-method shape — exactly how it bit Wheels'
//     URLFor(): declared `string params = ""`, a same-named route-params
//     struct in local, and a late Len(arguments.params) query-string check. ---
argShadowFixture = createObject("component", "ArgsShadowFixture");
assert("method: no query string when params was never passed",
	argShadowFixture.buildUrl(), "/posts");
assert("method: a passed params string reaches the query string intact",
	argShadowFixture.buildUrl(params = "page=2"), "/posts?page=2");

suiteEnd();
</cfscript>
