<cfscript>
suiteBegin("GH ##465 follow-up: nested local stores in a loop do not reset the frame");
// Wheels' model validation registration: v0.712.0 looped forever here (the loop
// counter was rolled back by a stale `local` snapshot). Values verified on Lucee 7.1.

function loopNestedDuplicate(required string a) {
	var n = 0;
	for (var i = 1; i <= 3; i++) {
		n++;
		if (n > 10) { return "RUNAWAY"; }
		local.v = {};
		local.v.args = Duplicate(arguments);
	}
	return n & "/" & local.v.args.a & "/" & structKeyList(local.v);
}
assert("Duplicate(arguments) into a nested local key, in a loop", loopNestedDuplicate(a = "z"), "3/z/args");

function loopNestedPlain() {
	var n = 0;
	for (var i = 1; i <= 3; i++) {
		n++;
		if (n > 10) { return "RUNAWAY"; }
		local.v = {};
		local.v.x = i;
	}
	return n & "/" & local.v.x;
}
assert("plain nested local store in a loop", loopNestedPlain(), "3/3");

function loopNestedWithEscape(required string a) {
	var s = local;
	var n = 0;
	for (var i = 1; i <= 3; i++) {
		n++;
		if (n > 10) { return "RUNAWAY"; }
		local.v = {};
		local.v.args = Duplicate(arguments);
	}
	return n & "/" & s.v.args.a & "/" & (structKeyExists(s, "i") ? s.i : "noi");
}
assert("same loop in a frame whose local has escaped stays live and terminates", loopNestedWithEscape(a = "y"), "3/y/4");

function accumulate() {
	local.acc = {};
	for (var i = 1; i <= 3; i++) {
		local.acc["k" & i] = i;
		local.acc.last = i;
	}
	return listSort(structKeyList(local.acc), "textnocase");
}
assert("bracket and dotted nested stores accumulate across iterations", accumulate(), "k1,k2,k3,last");
suiteEnd();
</cfscript>
