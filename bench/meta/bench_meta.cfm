<cfscript>
n = url.n ?: 500;
t0 = getTickCount("nano");
for (r=1; r<=n; r++) {
  for (i=0; i<20; i++) {
    md = getComponentMetaData("cfcs.Svc#i#");
    x = md.functions.len() + (structKeyExists(md,"extends") ? md.extends.functions.len() : 0);
  }
}
t1 = getTickCount("nano");
writeOutput("calls=" & (n*20) & " us/call=" & numberFormat((t1-t0)/1000/(n*20), "0.0") & chr(10));
</cfscript>