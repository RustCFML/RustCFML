<cfscript>
// Pure-numeric kernel — dense arithmetic, no member access. Historically the
// should be ~100x faster than the interpreter here. Baseline for any
// future polymorphic-rep change to compare against: this number must not
// regress.
function kernel(n) {
    var t = 0.0;
    for (var i = 1; i <= n; i++) {
        t = t + sqr(i) + log(i + 1) + sin(i / 10.0) + cos(i / 10.0)
              + floor(i / 3.0) + abs(i - 50);
    }
    return t;
}
total = 0.0;
for (k = 1; k <= 1000; k++) { total = total + kernel(20000); }
writeOutput(total);

</cfscript>
