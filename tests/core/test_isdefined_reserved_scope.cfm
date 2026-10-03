<cfscript>
suiteBegin("Core: isDefined() on cgi/url/form ignores same-named request keys (GH ##458)");

// Wheels stores a CGI copy at `request.cgi`. The dotted-string lookup walked
// the generic chain and reached `request.cgi` before the real scope, so
// `isDefined("cgi.http_x_...")` turned false for headers that were present.
url.gh458probe = 1;
request.url = { other = 1 };
assert("url.x still found after request.url is assigned", isDefined("url.gh458probe"), true);
assert("url.x does not read request.url", isDefined("url.other"), false);
assert("request.url itself is still reachable", isDefined("request.url.other"), true);
structDelete(request, "url");
structDelete(url, "gh458probe");

form.gh458probe = 1;
request.form = { other = 1 };
assert("form.x still found after request.form is assigned", isDefined("form.gh458probe"), true);
assert("form.x does not read request.form", isDefined("form.other"), false);
structDelete(request, "form");
structDelete(form, "gh458probe");

if (structKeyExists(cgi, "server_port") && len(cgi.server_port)) {
	request.cgi = { gh458only = "copied" };
	assert("cgi.server_port still found after request.cgi is assigned", isDefined("cgi.server_port"), true);
	assert("cgi.x does not read request.cgi", isDefined("cgi.gh458only"), false);
	structDelete(request, "cgi");
}

suiteEnd();
</cfscript>
