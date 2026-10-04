<cfscript>
// A parameter default is bound exactly like a supplied argument: it lands in
// `arguments` and is NOT a local, also under localmode="modern" (verified on
// Lucee 7.1). RustCFML applied defaults through the general variable store,
// which in modern mode made every defaulted parameter a LOCAL, and cost ~200 ns
// per default (Lucee ~24) because every store re-scanned the parameter list.
suiteBegin("Parameter defaults bind like supplied arguments");

function pdbClassic(a, b = "dflt", c = a & "!") {
    return { local: structKeyList(local), args: structKeyList(arguments), b: b, argsB: arguments.b, c: c };
}
function pdbModern(a, b = "dflt") localmode="modern" {
    return { local: structKeyList(local), args: structKeyList(arguments), b: b, argsB: arguments.b };
}
function pdbModernRebind(a, b = "dflt") localmode="modern" {
    b = "changed";
    return { b: b, argsB: arguments.b, local: structKeyList(local) };
}
function pdbTyped(numeric n = "abc") { return n; }
function pdbClosureDefault(x, f = function(v) { return v * 2; }) { return f(x); }
function pdbMany(a, b1 = "", b2 = 0, b3 = false, b4 = [], b5 = {}, b6 = "six") {
    return [b1, b2, b3, arrayLen(b4), structCount(b5), b6, structCount(arguments)];
}

r = pdbClassic(1);
assert("classic: default not in local", r.local, "");
assert("classic: arguments lists every param", r.args, "a,b,c");
assert("classic: default readable bare", r.b, "dflt");
assert("classic: default readable via arguments", r.argsB, "dflt");
assert("classic: default may use an earlier param", r.c, "1!");
assert("classic: a supplied value wins", pdbClassic(1, "given").b, "given");

r = pdbModern(1);
assert("modern: default not in local", r.local, "");
assert("modern: default readable bare", r.b, "dflt");
assert("modern: default readable via arguments", r.argsB, "dflt");

r = pdbModernRebind(1);
assert("modern: a bare write after the default makes a local", r.local, "b");
assert("modern: the argument keeps the default", r.argsB, "dflt");
assert("modern: the bare read sees the local", r.b, "changed");

assertThrows("a default is type-checked like an argument", function() { pdbTyped(); });
assert("a closure default is callable", pdbClosureDefault(21), 42);
assert("a passed function overrides a closure default", pdbClosureDefault(21, function(v) { return v + 1; }), 22);

assert("many defaults, positional call", serializeJSON(pdbMany(1)), serializeJSON(["", 0, false, 0, 0, "six", 7]));
assert("many defaults, named call", serializeJSON(pdbMany(a = 1, b6 = "x", b2 = 5)), serializeJSON(["", 5, false, 0, 0, "x", 7]));

suiteEnd();
</cfscript>
