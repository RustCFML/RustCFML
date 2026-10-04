<cfscript>
u = new ColdboxUtil();
for (i=0;i<20;i++) { getComponentMetaData("cfcs.Svc#i#"); }
n = 200;
t0 = getTickCount("nano");
for (r=1;r<=n;r++) { for (i=0;i<20;i++) { md = u.getInheritedMetaData( "cfcs.Svc#i#" ); } }
t1 = getTickCount("nano");
for (r=1;r<=n;r++) { for (i=0;i<20;i++) { md2 = getComponentMetaData( "cfcs.Svc#i#" ); } }
t2 = getTickCount("nano");
writeOutput("getInheritedMetaData: " & numberFormat((t1-t0)/1000/(n*20),"0.0") & " us/call (functions=" & arrayLen(md.functions) & ") | bare getComponentMetaData: " & numberFormat((t2-t1)/1000/(n*20),"0.0") & " us" & chr(10));
</cfscript>