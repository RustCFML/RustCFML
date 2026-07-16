<cfscript>
// In-place struct member mutators (delete/insert/clear) invoked on a reserved
// SCOPE must mutate the live scope, not a throwaway copy. The `request` scope
// in particular was loaded as a snapshot, so `request.delete("x")` silently
// no-op'd — the change never reached the live scope.
//
// Repro family: Preside admin login. `LoginService._persistUserSession` calls
// `request.delete("__presideCmsAminUserDetails")` to clear a cached user-details
// struct; the no-op left a stale EMPTY struct cached, so the post-login
// `getLoggedInUserDetails().login_id` read threw "Variable 'login_id' is
// undefined". Lucee-verified (scopes are references in CFML).
suiteBegin("Scope member mutators");

function testRequestScope() {
	request.rk = "v";
	request.delete("rk");
	assertFalse( "request.delete removes key", StructKeyExists( request, "rk" ) );

	request.insert("ik", 42);
	assert( "request.insert adds key", request.ik, 42 );

	// NB: `request.clear()` is deliberately NOT tested here — it would wipe the
	// harness's own request-scoped counters. It shares the same fixed code path
	// as delete/insert (in-place scope mutation writeback).
}
testRequestScope();

// Same must hold when the mutation happens inside one method and is observed by
// another call in the same request (the exact Preside shape: cache set, then
// deleted, then re-read).
function seed() { request.cache = { stale = true }; }
function drop() { request.delete("cache"); }
function present() { return StructKeyExists( request, "cache" ); }
seed();
assertTrue( "seeded cache present", present() );
drop();
assertFalse( "request.delete across method calls clears cache", present() );

// Plain structs must be unaffected (in-place mutation already worked).
plain = { x = 1, y = 2 };
plain.delete("x");
assertFalse( "plain struct .delete still works", StructKeyExists( plain, "x" ) );
assert( "plain struct retains other keys", plain.y, 2 );

// Returning a bare SCOPE from a function must hand back a live reference, not a
// snapshot — writes through the returned handle must reach the real scope
// (Lucee-verified; scopes are references in CFML). This is the exact shape of
// Masa CMS's servletEvent.getScope()/setValue(): `theScope = getScope("request");
// theScope[key] = obj;` then `getValue(key)` reads the live scope. When `request`
// was returned as a snapshot the stored HandlerFactory was lost, so
// getHandler("...").handle() called .handle() on a null → front-end index.cfm 500.
function getReqScope() { return request; }
scopeAlias = getReqScope();
scopeAlias["storedViaAlias"] = "propagated";
assertTrue( "write via returned request handle reaches scope",
	StructKeyExists( request, "storedViaAlias" ) );
assert( "value written via returned request handle is readable",
	request.storedViaAlias, "propagated" );

// Storing a COMPONENT through the returned handle must survive round-trip as an
// object (the HandlerFactory case — a simple-value snapshot would drop it).
function getReqScope2() { return request; }
compAlias = getReqScope2();
compAlias["storedComp"] = createObject("component","scope_ref_probe");
assertTrue( "component stored via alias is an object", IsObject( request.storedComp ) );
assert( "component stored via alias is callable", request.storedComp.ping(), "pong" );

// Same for the application scope (already correct — guards against regression).
function getAppScope() { return application; }
appAlias = getAppScope();
appAlias["storedViaAliasApp"] = "app-propagated";
assertTrue( "write via returned application handle reaches scope",
	StructKeyExists( application, "storedViaAliasApp" ) );

// NB: wholesale `request = {...}` replacement is deliberately NOT tested here —
// it would wipe the harness's own request-scoped counters (same hazard as
// request.clear() above). Its self-alias writeback guard is exercised by the
// per-key member writes above.

suiteEnd();
</cfscript>
