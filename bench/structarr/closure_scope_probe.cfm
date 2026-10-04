<cfscript>
function makeClosure() { var defined = "D"; return function() { return (isDefined("defined") ? defined : "-") & "/" & (isDefined("callerOnly") ? callerOnly : "-"); }; }
function callIt(c) { var callerOnly = "C"; return c(); }
writeOutput("closure called from another function: " & callIt(makeClosure()) & chr(10));
function sameFrame() { var outer = "O"; var late = ""; var g = function() { return outer & "/" & late; }; late = "L"; return g(); }
writeOutput("closure sees a later assignment in its defining frame: " & sameFrame() & chr(10));
function writes() { var n = 1; var g = function() { n = n + 1; }; g(); g(); return n; }
writeOutput("closure writes its defining frame's var: " & writes() & chr(10));
function passFn(h) { var callerOnly = "C2"; return h(); }
function viaArrayEach() { var acc = ""; [1,2].each(function(v){ acc &= v; }); return acc; }
writeOutput("each closure accumulates into outer var: " & viaArrayEach() & chr(10));
</cfscript>
