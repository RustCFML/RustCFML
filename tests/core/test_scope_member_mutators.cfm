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

suiteEnd();
</cfscript>
