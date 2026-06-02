<cfscript>
suiteBegin("Core: undeclared named arguments keep their names");

// ============================================================
// Background
// ============================================================
// Calling a function with MORE named arguments than it declares is standard
// CFML on Lucee 5/6/7, Adobe CF 2018-2025, and BoxLang: the extra named args
// remain reachable BY NAME in the `arguments` scope. This underpins the common
// framework pattern of declaring a few params, accepting arbitrary extra named
// args, and forwarding `arguments` on to another method.
//
// On RustCFML the extra (undeclared) named args are stored POSITIONALLY — the
// `arguments` scope gets numeric keys (1, 2, 3, ...) instead of the names, so
// StructKeyExists(arguments, "<name>") is false and the values can't be read by
// name or forwarded.
//
// This blocks the Wheels boot: vendor/wheels/Global.cfc $createObjectFromRoot
// declares path/fileName/method, is called with extra named args (pluginPath,
// deletePluginDirectories, ...), and forwards `arguments` to the target $init —
// which then receives an empty pluginPath, sending the plugin-directory scan to
// the wrong path.
// ============================================================

o = createObject("component", "UndeclaredArgFixture");
assert("extra named args are reachable by name in the arguments scope",
	o.probe(), "a=A,b=B,c=C");

suiteEnd();
</cfscript>
