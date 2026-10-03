<cfscript>
suiteBegin("local read as a value is the live scope (GH ##465)");

// Every expected value below was taken from Lucee 7.1.0.204.

function liveWrite() { var x = 1; var s = local; s.w = 2; return isDefined("local.w") & "/" & (structKeyExists(local, "w") ? "y" : "n"); }
assert("a write through the handle creates a local", liveWrite(), "true/y");

function laterVar() { var s = local; var y = 3; return structKeyExists(s, "y") ? "y" : "n"; }
assert("a local declared after the read shows in the handle", laterVar(), "y");

function deleteThrough() { var x = 1; var s = local; structDelete(s, "x"); return isDefined("x") ? "still" : "gone"; }
assert("structDelete through the handle deletes the local", deleteThrough(), "gone");

function updateThrough() { var x = 1; var s = local; s.x = 5; return x; }
assert("an update through the handle is seen by a bare read", updateThrough(), 5);

function updateFrame() { var x = 1; var s = local; x = 7; return s.x; }
assert("a bare write is seen through the handle", updateFrame(), 7);

function shadowParam(p) { var s = local; s.p = 9; return p & "/" & (structKeyExists(local, "p") ? "y" : "n") & "/" & arguments.p; }
assert("a handle key named like a parameter shadows it; arguments keeps the caller's value", shadowParam("a"), "9/y/a");

function fillIt(st) { st.filled = true; }
function passToHelper() { fillIt(local); return isDefined("local.filled") ? "y" : "n"; }
assert("a helper handed `local` fills the caller's scope", passToHelper(), "y");

function selfContained() { var s = local; return structCount(s); }
assert("the scope contains the variable holding it", selfContained(), 1);

function viaBifs() { var s = getVariable("local"); s.z = 1; var e = evaluate("local"); e.q = 2; return isDefined("local.z") & isDefined("local.q"); }
assert("getVariable('local') and evaluate('local') are the live scope too", viaBifs(), "truetrue");

function loopCounter() { var x = 1; var s = local; for (var i = 1; i <= 3; i++) { x++; } return s.x & "/" & s.i; }
assert("locals changed in a loop are current in the handle", loopCounter(), "4/4");

function secondRead() { var s = local; s.k = 1; var s2 = local; return isDefined("s2.k") & "/" & structCount(s2); }
assert("a second read is the same scope", secondRead(), "true/3");

function selfEquals() { var s = local; return s.equals(s); }
assertTrue("equals() on the self-referential scope terminates", selfEquals());

function explicitLocalParam(p) { local.p = 9; return p & "/" & arguments.p; }
assert("explicit local.p on a parameter still leaves arguments.p alone", explicitLocalParam("a"), "9/a");

function argsFirst(p) { var s = local; s.p = 9; return arguments.p & "/" & p; }
assert("arguments.p read before the bare name after a handle shadow", argsFirst("a"), "a/9");

suiteEnd();
</cfscript>
