<!--- §106: the arguments scope IS a parameter's storage (Lucee 7.1 parity).
      structDelete(arguments, "a") / structClear(arguments) clear the bare name. --->
<cfscript>
suiteBegin("structDelete(arguments) clears the bare parameter name");

function sda_delete(a, b) {
    var r = {};
    structDelete(arguments, "a");
    try { r.bareRead = a; r.threw = false; } catch (any e) { r.threw = true; }
    r.isDef = isDefined("a");
    r.count = structCount(arguments);
    r.keyExists = structKeyExists(arguments, "a");
    // A later bare write is an ordinary variable — it must NOT resurrect the key.
    a = "REWRITTEN";
    r.afterBare = a;
    r.afterBareKey = structKeyExists(arguments, "a");
    r.afterBareCount = structCount(arguments);
    // A scoped write restores both views.
    arguments.a = "VIA_SCOPE";
    r.afterScoped = a;
    r.afterScopedArg = arguments.a;
    r.afterScopedCount = structCount(arguments);
    return r;
}
res = sda_delete(1, 2);
assertTrue("bare read of the deleted param throws", res.threw);
assertFalse("isDefined() false after delete", res.isDef);
assert("count drops 2 -> 1", res.count, 1);
assertFalse("structKeyExists false after delete", res.keyExists);
assert("later bare write is readable", res.afterBare, "REWRITTEN");
assertFalse("later bare write does not resurrect the arguments key", res.afterBareKey);
assert("count still 1 after the bare write", res.afterBareCount, 1);
assert("arguments.a = restores the bare name", res.afterScoped, "VIA_SCOPE");
assert("arguments.a = restores the scope key", res.afterScopedArg, "VIA_SCOPE");
assert("count back to 2", res.afterScopedCount, 2);

// A bare write to a DELETED parameter is an ordinary variable write, so every
// function below uses its own parameter names: on Lucee (classic localmode) the
// `a = "REWRITTEN"` above landed in the page variables scope.
function sda_dynamic(sdaDyn1, sdaDyn2) {
    var k = "sdaDyn1";
    structDelete(arguments, k);
    return isDefined("sdaDyn1") & "|" & structCount(arguments);
}
assert("dynamic key expression", sda_dynamic(1, 2), "false|1");

function sda_localShadow(a) {
    local.a = "LOCAL";
    structDelete(arguments, "a");
    return a & "|" & structCount(arguments);
}
assert("an explicit local.a is a separate slot and survives", sda_localShadow(1), "LOCAL|0");

function sda_varShadow(a) {
    var a = "VAR";
    structDelete(arguments, "a");
    return a & "|" & structCount(arguments);
}
assert("a var a is a separate slot and survives", sda_varShadow(1), "VAR|0");

function sda_omitted(a, b) {
    structDelete(arguments, "a");
    var r = structKeyExists(arguments, "b") & "|" & structCount(arguments);
    structDelete(arguments, "b");
    return r & "|" & structCount(arguments);
}
assert("a declared-but-omitted param keeps its entry until deleted", sda_omitted(1), "false|1|0");

function sda_clear(sdaClr1, sdaClr2) {
    var r = {};
    structClear(arguments);
    try { r.bareA = sdaClr1; r.threwA = false; } catch (any e) { r.threwA = true; }
    r.isDefB = isDefined("sdaClr2");
    r.count = structCount(arguments);
    return r;
}
res = sda_clear(1, 2);
assertTrue("structClear(arguments): bare a throws", res.threwA);
assertFalse("structClear(arguments): isDefined(b) false", res.isDefB);
assert("structClear(arguments): count 0", res.count, 0);

function sda_bareWriteOmitted(a, b) {
    b = 5;
    return structKeyExists(arguments, "b") & "|" & arguments.b & "|" & structCount(arguments);
}
assert("a bare write to an omitted param still lands in arguments", sda_bareWriteOmitted(1), "true|5|2");

function sda_deleteOther(a) {
    var r = {};
    structDelete(arguments, "nope");
    r.a = a;
    r.count = structCount(arguments);
    return r;
}
res = sda_deleteOther(7);
assert("deleting a missing key leaves the params alone", res.a, 7);
assert("deleting a missing key leaves the count alone", res.count, 1);

suiteEnd();
</cfscript>
