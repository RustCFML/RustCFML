<cfscript>
function try_(n){ try { return getComponentMetaData(n).path; } catch (any e) { return "ERR " & e.message; } }
for (n in ["m.Thing","m..Thing","/m/Thing","/m//Thing","lib.Thing","lib..Thing"]) writeOutput(n & " => " & try_(n) & chr(10));
</cfscript>
