<cfscript>
suiteBegin("Core: evaluate() of scope names + reflective builtins as callbacks (GH ##370, ##371, ##449)");

// --- evaluate("<scope>") returns that scope, live, as on Lucee 7.1 ---
function _gh371vars() {
	var s = evaluate("variables");
	s["gh371probe"] = "yes";
	return structKeyExists(variables, "gh371probe") ? "LIVE" : "COPY";
}
function _gh371args(a = "A") {
	var s = evaluate("arguments");
	s["injected"] = "yes";
	return structKeyExists(arguments, "injected") ? "LIVE" : "COPY";
}
function _gh371argsDynamic(a = "A") {
	var e = "arguments";
	var s = evaluate(e);
	s["injected"] = "yes";
	return structKeyExists(arguments, "injected") ? "LIVE" : "COPY";
}
assert("evaluate('variables') is the live scope", _gh371vars(), "LIVE");
assert("evaluate('arguments') is the live scope", _gh371args(), "LIVE");
assert("evaluate(dynamic 'arguments') is the live scope", _gh371argsDynamic(), "LIVE");
structDelete(variables, "gh371probe");

function _gh370local() {
	var x = 1;
	var l = evaluate("local");
	return isStruct(l) & ":" & structKeyExists(l, "x");
}
function _gh370localKey() { var x = 5; return evaluate("local.x"); }
function _gh370localDynamic(p = 1) {
	var x = 9;
	var e = "local";
	return listSort(structKeyList(evaluate(e)), "textnocase");
}
assert("evaluate('local') is the local scope", _gh370local(), "true:true");
assert("evaluate('local.x')", _gh370localKey(), 5);
assert("evaluate(dynamic 'local') lists only locals, not params", _gh370localDynamic(), "e,x");
function _gh370expr() { var x = 2; return evaluate("x * 3") & evaluate("true") & evaluate("'s'"); }
assert("evaluate of an expression still evaluates", _gh370expr(), "6trues");

// --- a reflective builtin passed as a callback resolves in the caller's scope ---
if (isRustCFML()) {
	gh449 = { a: { b: 42 } };
	assert("member map(structGet) at page level", serializeJSON([ "gh449.a.b" ].map(structGet)), "[42]");
	assert("arrayMap(structGet) at page level", serializeJSON(arrayMap([ "gh449.a.b" ], structGet)), "[42]");
	function _gh449inFunction() {
		var s = { a: { b: 7 } };
		return serializeJSON([ "s.a.b" ].map(structGet)) & serializeJSON(arrayMap([ "s.a.b" ], getVariable));
	}
	assert("callbacks inside a function see its locals", _gh449inFunction(), "[7][7]");
	assert("member filter with a builtin callback", serializeJSON([ "gh449", "nope449" ].filter(isDefined)), '["gh449"]');
	structDelete(variables, "gh449");
}

suiteEnd();
</cfscript>
