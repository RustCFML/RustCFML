<cfscript>
// String / polymorphic kernel — the shape a pure-numeric compiler could never
// surface v0.88.0 will measure (coverage signal) and v0.90.0 will start
// targeting (boxed `+` / concat). Expected baseline: ~1.0× (interpreter
// only).
function buildLine(prefix, n) {
    var s = prefix;
    for (var i = 1; i <= n; i++) {
        s = s & "-" & i;
    }
    return s;
}
total = "";
for (k = 1; k <= 5000; k++) { total = buildLine("row" & k, 300); }
writeOutput(len(total));

</cfscript>
