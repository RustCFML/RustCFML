<!--- GH #351 — a page template and a CFC pseudo-constructor have NO `local` scope.

     This engine used to expose one at page level that read straight through to
     the page's `variables`, so `local.foo = 1` in a template silently wrote a
     page variable and `local.someUnrelatedPageVar` read back a value. Lucee has
     no local scope there at all: `local` is an ordinary variable name, so
     `local.foo = 1` creates `variables.local` and naming `local` before that
     throws "variable [local] doesn't exist".

     The harmful direction was ours-works/Lucee-breaks: code written here
     deployed elsewhere and failed only then.

     Behaviour below measured on Lucee 7.1.0.204. The companion file
     tests/core/test_local_at_template_scope.cfm covers the auto-vivified-struct
     side of the same rule; this one covers the isolation properties.

     A note if you extend this: naming `local` directly aborts the file on Lucee
     when it is undefined, so every "before" probe has to go through
     isDefined() or a try/catch. --->
<cfscript>
suiteBegin("Core: page scope has no local scope (GH ##351)");

_pageOnlyVar = "pv";

// --- before anything writes it, `local` simply does not exist ---
assertFalse("isDefined('local') is false at page scope", isDefined("local"));
assertFalse("a page variable is NOT visible as local.*", isDefined("local._pageOnlyVar"));
assertFalse("no `local` key in the page variables scope", structKeyExists(variables, "local"));

// Naming it does not yield a scope. On Lucee (and here, in isolation) it throws
// outright; the assertion is written against the shape rather than the throw
// because the full-suite run can leave a UDF literally named `local` reachable
// as a bare name — a CFC's methods stay visible as bare names in later templates
// on this engine, for `local` and `normal` alike — GH ##360, a separate
// divergence from anything this file is about. Either way the point holds: naming `local`
// at page scope must NOT hand back a scope struct.
_bareLocal = "threw";
try { _bareLocal = isStruct( local ) ? "struct" : "other"; } catch (any e) { _bareLocal = "threw"; }
assertFalse("naming `local` at page scope does not yield a scope struct", _bareLocal EQ "struct");

// --- after a write it is an ORDINARY page variable, not a scope ---
local.pageLevelKey = "written";
assertTrue("isDefined('local') is true once assigned", isDefined("local"));
assertTrue("the write lands in `variables`, as a plain struct",
	structKeyExists(variables, "local"));
assert("read back through variables.local", variables.local.pageLevelKey, "written");
assert("read back through the bare name", local.pageLevelKey, "written");

// Crucially it did NOT splice its keys into the page scope, and page variables
// still do not appear inside it.
assertFalse("`local.pageLevelKey` did not create a page variable `pageLevelKey`",
	isDefined("variables.pageLevelKey"));
assertFalse("page variables are still not visible as local.*",
	structKeyExists(local, "_pageOnlyVar"));

// --- inside a function, `local` is the function scope, exactly as before ---
_fnProbe = function() {
	local.fnKey = "inFunction";
	return {
		isScope:     NOT structKeyExists(local, "pageLevelKey"),
		hasOwn:      structKeyExists(local, "fnKey"),
		isDefinedOk: isDefined("local.fnKey")
	};
};
_fn = _fnProbe();
assertTrue("a function's `local` does NOT see the page's `local` struct", _fn.isScope);
assertTrue("a function's `local` holds its own keys", _fn.hasOwn);
assertTrue("isDefined('local.x') works inside a function", _fn.isDefinedOk);
assertFalse("the function's local key did not leak to the page", structKeyExists(local, "fnKey"));

// --- `for ( local.X in … )` at page level ---
// The loop variable is `variables.local.X` here, not a bare `X`. Codegen used to
// strip the `local.` prefix unconditionally (a function-scope normalisation), so
// the loop WROTE `X` while the body READ `local.X` — the loop then ran the right
// number of times over an unset variable, silently doing nothing. That is not
// hypothetical: it is exactly how Wheels' `WheelsTest.cfc` pseudo-constructor
// (`for (local.method in local.methods)`) stopped injecting any methods at all,
// and it surfaced 75 spec errors away from the loop, as missing methods.
local.items = [ "a", "b", "c" ];
_seen = "";
for ( local.item in local.items ) {
	_seen = listAppend( _seen, local.item );
}
assert("for-in over local.X binds the loop variable", _seen, "a,b,c");
assertFalse("the loop variable did not leak as a bare page variable",
	isDefined("variables.item"));

// --- a CFC pseudo-constructor behaves like a page, not like a method ---
_pc = createObject("component", "core.LocalAtPseudoCtor");
assertFalse("pseudo-constructor: isDefined('local') is false before any write",
	_pc.getIsDefinedBefore());
assertTrue("pseudo-constructor: `local.x = v` creates variables.local",
	_pc.getCreatedVariablesLocal());
assert("pseudo-constructor: the value reads back", _pc.getReadBack(), 1);
assertFalse("pseudo-constructor: the key did NOT become a component variable",
	_pc.getLeakedToVariables());
assert("pseudo-constructor: for-in over local.X binds the loop variable",
	_pc.getLoopSeen(), "1,2,3");
assertTrue("a METHOD on the same component still has a real local scope",
	_pc.methodHasOwnLocalScope());

suiteEnd();
</cfscript>
