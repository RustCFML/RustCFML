<cfscript>
n = url.n ?: 200;
o = new Big150();
t0 = getTickCount("nano"); for (i=1;i<=n;i++) { o = new Big150(); } t1 = getTickCount("nano");
writeOutput("new x#n#: " & numberFormat((t1-t0)/1000/n,"0.0") & " us/new" & chr(10));
</cfscript>