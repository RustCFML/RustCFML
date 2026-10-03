<cfscript>
n = 2000;
function tm(label, fn) { fn(); var t0 = getTickCount("nano"); for (var i=1;i<=n;i++) { fn(); } writeOutput(label & ": " & numberFormat((getTickCount("nano")-t0)/1000/n,"0.0") & " us/new" & chr(10)); }
f = new bench.p7.Factory();
tm("page unqualified createObject", function(){ return createObject("component","Plain20"); });
tm("page dotted createObject     ", function(){ return createObject("component","bench.p7.Plain20"); });
tm("page dotted new              ", function(){ return new bench.p7.Plain20(); });
tm("cfc unqualified new          ", function(){ return f.makeRel(); });
tm("cfc dotted new               ", function(){ return f.makeDotted(); });
tm("cfc unqualified createObject ", function(){ return f.makeCreateRel(); });
</cfscript>
