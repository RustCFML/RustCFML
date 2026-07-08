<cfscript>
suiteBegin("Core: scope-named parameter must not bind the live scope struct (GH ##256)");

// Regression (GitHub #256, v0.423.0): a declared parameter whose NAME matches a
// built-in scope (cookie/session/request/server/application/variables) bound the
// LIVE scope struct instead of its declared default when the caller omitted the
// argument. url/form/cgi and ordinary names were unaffected. On Lucee/ACF an
// omitted defaulted parameter ALWAYS binds arguments.<name> to the declared
// default; a scope name has no special meaning in a parameter list, because
// local/arguments precede every built-in scope in the CFML cascade.
//
// Two coordinated engine bugs: (1) StoreLocal redirected a `request`/`session`/
// `application`/`variables` store into the live scope, silently dropping a
// non-struct default; (2) LoadLocal returned the scope struct unconditionally
// for a scope-named identifier, before consulting local/arguments.

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
		local.v = arguments[local.k] ?: "NULL";
		local.report[local.k] = IsSimpleValue(local.v) ? "simple" : "notsimple";
	}
	return local.report;
}

report = probe();
for (name in report) {
	assert("omitted scope-named param '#name#' binds its default (not the scope)", report[name], "simple");
}

// Explicitly-passed value must win.
function passer(string cookie = "DEF") { return arguments.cookie; }
assert("omitted cookie param takes default", passer(), "DEF");
assert("passed cookie param overrides default", passer(cookie = "hello"), "hello");

// A defaulted scope-named param must NOT leak the live scope — even when the
// scope is non-empty. Plant a marker in the request scope, then confirm an
// omitted `request` param reports the default rather than the scope struct.
request.marker256 = "PLANTED";
function shadowsRequest(string request = "DEF") { return arguments.request; }
assert("defaulted request param does not leak live request scope", shadowsRequest(), "DEF");

// A function WITHOUT a shadowing param still reads the real scope normally.
function readsRequest() { return request.marker256; }
assert("real request scope still readable when no param shadows it", readsRequest(), "PLANTED");

// Bare scope reads are unregressed when nothing shadows them.
function bareScopes() { return IsStruct(cgi) && IsStruct(server) && IsStruct(cookie); }
assertTrue("bare scope reads still return structs without a shadowing param", bareScopes());

// A `var`-declared local named after a scope shadows the scope too.
function varLocal() { var cookie = "LOCAL"; return cookie; }
assert("var-declared scope-named local shadows the scope", varLocal(), "LOCAL");

suiteEnd();
</cfscript>
