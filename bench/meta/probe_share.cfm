<cfscript>
md = getComponentMetaData("cfcs.Svc1"); md.zzz = 1; md.functions[1].qq = 2;
md2 = getComponentMetaData("cfcs.Svc1");
writeOutput("path-form shared top: " & structKeyExists(md2,"zzz") & " / shared fn: " & structKeyExists(md2.functions[1],"qq") & chr(10));
o = new cfcs.Svc2(); m = getMetaData(o); m.yy = 1; m2 = getMetaData(o);
writeOutput("instance-form shared: " & structKeyExists(m2,"yy") & chr(10));
m3 = getComponentMetaData("cfcs.Svc2");
writeOutput("instance vs path same cache: " & structKeyExists(m3,"yy") & chr(10));
</cfscript>
