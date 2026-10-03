<cfscript>
a = getComponentMetaData("cfcs.Svc1"); b = getMetaData(new cfcs.Svc1());
writeOutput("path: " & listSort(structKeyList(a),"textnocase") & chr(10) & "inst: " & listSort(structKeyList(b),"textnocase") & chr(10));
</cfscript>
