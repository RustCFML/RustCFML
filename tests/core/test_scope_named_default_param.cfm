<cfscript>
suiteBegin("Core: built-in scope names are reserved for bare reads (GH ##312, ##256)");

// Two rules that look contradictory and are not. Both verified against Lucee 7.0.4.
//
//   1. GH ##312 — a built-in scope name is RESERVED for a BARE read. A same-named
//      parameter or `var` local does NOT shadow it: bare `cookie` is always the
//      cookie scope. The shadowing value stays reachable through its explicit
//      qualifier (`arguments.cookie`).
//   2. GH ##256 — `arguments.<name>` binds an omitted parameter's declared
//      DEFAULT, not the live scope. This is a different path from the bare read.
//
// This file previously asserted the OPPOSITE of rule 1 (a scope-named parameter
// shadowing the scope), which is Adobe CF's behaviour. It passed on RustCFML and
// ERRORED on Lucee 7 with "Can't cast Complex Object Type [COOKIE scope] to
// String" — a test failing on the reference engine being the clearest sign we had
// taken the wrong fork. Lucee is the reference, so rule 1 is now Lucee's.

// ---------------------------------------------------------------------------
// Rule 2 (GH ##256): arguments.<name> gets the DEFAULT for an omitted param.
// ---------------------------------------------------------------------------
function probe(
	string cookie = "DEFAULT",
	string session = "DEFAULT",
	string url = "DEFAULT",
	string form = "DEFAULT",
	string request = "DEFAULT",
	string server = "DEFAULT",
	string cgi = "DEFAULT",
	string application = "DEFAULT",
	string params = "DEFAULT"
) {
	local.report = {};
	for (local.k in ["cookie", "session", "url", "form", "request", "server", "cgi", "application", "params"]) {
		local.report[local.k] = arguments[local.k] ?: "NULL";
	}
	return local.report;
}

report = probe();
for (name in report) {
	assert("omitted scope-named param '#name#' binds its default via arguments", report[name], "DEFAULT");
}

// Explicitly-passed value wins, and is visible through arguments.
function passer(string cookie = "DEF") { return arguments.cookie; }
assert("omitted cookie param takes default", passer(), "DEF");
assert("passed cookie param overrides default", passer(cookie = "hello"), "hello");

// A live, non-empty scope must not leak into arguments for an omitted param.
request.marker256 = "PLANTED";
function shadowsRequest(string request = "DEF") { return arguments.request; }
assert("omitted request param does not leak the live request scope", shadowsRequest(), "DEF");

// The default is seeded WITHOUT reading the parameter back by bare name — that
// read-back is what made a scope-named param's default become the scope struct
// once rule 1 landed. Covered for a declared function, a closure and an arrow,
// because all three emit their own default preamble.
function closureDefault() { var f = function(cookie = "CLO") { return arguments.cookie; }; return f(); }
arrowDefault = (cookie = "ARR") => arguments.cookie;
assert("closure default is not the scope", closureDefault(), "CLO");
assert("arrow default is not the scope", arrowDefault(), "ARR");

// A default that references an earlier param still resolves (GH ##240).
function selfRef(x = 1, y = x + 1) { return "#x#/#y#"; }
assert("a default may reference an earlier param", selfRef(), "1/2");

// ---------------------------------------------------------------------------
// Rule 1 (GH ##312): a BARE scope name is always the scope.
// ---------------------------------------------------------------------------
// A parameter named after a scope does not shadow it.
function bareWithParam(cookie) { return isStruct(cookie) ? "scope" : "param"; }
assert("a scope-named param does not shadow the bare read", bareWithParam("PASSED"), "scope");

// Nor does a `var` local, nor a `local.` assignment.
function bareWithVar() { var cookie = "LOCAL"; return isStruct(cookie) ? "scope" : "local"; }
function bareWithLocalDot() { local.cookie = "LOCAL"; return isStruct(cookie) ? "scope" : "local"; }
assert("a var-declared scope-named local does not shadow the bare read", bareWithVar(), "scope");
assert("a local.-assigned scope name does not shadow the bare read", bareWithLocalDot(), "scope");

// An ordinary name is unaffected — the rule is about scope names only.
function ordinaryVar() { var notascope = "LOCAL"; return notascope; }
assert("an ordinary var local is unaffected", ordinaryVar(), "LOCAL");

// The rule is uniform across every built-in scope, with no per-scope exception.
function sRequest(request)         { return isStruct(request); }
function sCookie(cookie)           { return isStruct(cookie); }
function sUrl(url)                 { return isStruct(url); }
function sForm(form)               { return isStruct(form); }
function sCgi(cgi)                 { return isStruct(cgi); }
function sSession(session)         { return isStruct(session); }
function sApplication(application) { return isStruct(application); }
function sServer(server)           { return isStruct(server); }
function sVariables(variables)     { return isStruct(variables); }
assertTrue("bare request is the scope",     sRequest("X"));
assertTrue("bare cookie is the scope",      sCookie("X"));
assertTrue("bare url is the scope",         sUrl("X"));
assertTrue("bare form is the scope",        sForm("X"));
assertTrue("bare cgi is the scope",         sCgi("X"));
assertTrue("bare session is the scope",     sSession("X"));
assertTrue("bare application is the scope", sApplication("X"));
assertTrue("bare server is the scope",      sServer("X"));
assertTrue("bare variables is the scope",   sVariables("X"));

// Bare scope reads with nothing shadowing them are unregressed.
function bareScopes() { return isStruct(cgi) && isStruct(server) && isStruct(cookie); }
assertTrue("bare scope reads still return structs with no shadowing param", bareScopes());

function readsRequest() { return request.marker256; }
assert("the real request scope is readable when nothing shadows it", readsRequest(), "PLANTED");

// ---------------------------------------------------------------------------
// The consequence that made GH ##312 a crash rather than a divergence: isDefined
// and the bare read must give the SAME answer. This is the Wheels middleware
// shape — a handler whose parameter is named `request`.
// ---------------------------------------------------------------------------
request.wheels = { tenant = { id = "FROM-SCOPE" } };

function handler(required struct request) {
	return isDefined("request.wheels.tenant") ? request.wheels.tenant.id : "NOT-DEFINED";
}
// The argument carries no `wheels` key at all: isDefined answered from the scope
// while the read answered from the argument, so this threw
// "Variable 'wheels' is undefined" despite the guard.
assert("guarded scope read agrees with isDefined (arg lacks the key)",
       handler({ cgi = { server_name = "example.com" } }), "FROM-SCOPE");
// Even when the argument DOES have the key, the bare read is the scope.
assert("bare read is the scope even when the argument has the key",
       handler({ wheels = { tenant = { id = "FROM-ARG" } } }), "FROM-SCOPE");
// ...and the argument remains reachable, explicitly.
function viaArguments(required struct request) { return arguments.request.wheels.tenant.id; }
assert("the shadowing argument is still reachable via arguments",
       viaArguments({ wheels = { tenant = { id = "FROM-ARG" } } }), "FROM-ARG");

structDelete(request, "wheels");
structDelete(request, "marker256");

suiteEnd();
</cfscript>
