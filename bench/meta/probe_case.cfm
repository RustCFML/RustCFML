<cfscript>
u = getComponentMetaData("CFCS.SVC1"); l = getComponentMetaData("cfcs.Svc1");
writeOutput("upper.name=" & u.name & " upper.fullname=" & u.fullname & " lower.name=" & l.name & chr(10));
i = getMetaData(new cfcs.Svc1()); writeOutput("inst.name=" & i.name & chr(10));
</cfscript>
