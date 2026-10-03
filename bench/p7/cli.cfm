<cfscript>
function bench(name) { var o = createObject("component", name); var n = 2000; var t0 = getTickCount("nano"); for (var i=1;i<=n;i++) { o = createObject("component", name); } return numberFormat((getTickCount("nano")-t0)/1000/n, "0.0"); }
for (nm in ["Plain20","Plain150","Big150"]) writeOutput(nm & ": " & bench(nm) & " us/new" & chr(10));
</cfscript>