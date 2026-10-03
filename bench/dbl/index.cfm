<cfscript>
writeOutput("path-form: " & getComponentMetaData("m.Thing").path & chr(10));
writeOutput("instance : " & getMetaData(new m.Thing()).path & chr(10));
</cfscript>
