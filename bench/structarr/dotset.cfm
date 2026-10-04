<cfscript>
function t(label, fn, n) { fn(100); var t0 = getTickCount("nano"); fn(n); out &= left(label & repeatString(" ", 40), 40) & numberFormat((getTickCount("nano") - t0) / n, "0") & chr(10); }
out = ""; N = url.n ?: 300000;
variables.cache = {};
t("var s; s.k = i", function(n){ var s = {}; for (var i=1;i<=n;i++) { s.k = i; } }, N);
t("var s; s.a.b = i (nested)", function(n){ var s = {a={}}; for (var i=1;i<=n;i++) { s.a.b = i; } }, N);
t("local.r = {} once; local.r.k = i", function(n){ local.r = {}; for (var i=1;i<=n;i++) { local.r.k = i; } }, N);
t("local.k = i", function(n){ for (var i=1;i<=n;i++) { local.k = i; } }, N);
t("arguments-struct a.k = i", function(n, struct a = {}){ for (var i=1;i<=n;i++) { arguments.a.k = i; } }, N);
t("param struct a.k = i", function(n, struct a = {}){ for (var i=1;i<=n;i++) { a.k = i; } }, N);
t("variables.cache.k = i", function(n){ for (var i=1;i<=n;i++) { variables.cache.k = i; } }, N);
t("variables.x = i", function(n){ for (var i=1;i<=n;i++) { variables.x = i; } }, N);
t("read local.r.k", function(n){ local.r = {k=1}; var x = 0; for (var i=1;i<=n;i++) { x = local.r.k; } }, N);
t("read variables.cache.k", function(n){ variables.cache.k = 1; var x = 0; for (var i=1;i<=n;i++) { x = variables.cache.k; } }, N);
t("empty loop", function(n){ for (var i=1;i<=n;i++) { } }, N);
writeOutput(out);
</cfscript>
