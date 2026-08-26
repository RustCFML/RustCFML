<cfscript>
fns = getFunctionList();
writeOutput( "total functions the engine reports: " & structCount( fns ) & chr(10) );
writeOutput( "demoGreet present?  " & structKeyExists( fns, "demoGreet" ) & chr(10) );
writeOutput( "slugify present?    " & structKeyExists( fns, "slugify" ) & chr(10) );
writeOutput( "attributed to:      '" & fns.demoGreet & "'" & chr(10) );
writeOutput( "a compiled-in one:  '" & fns.ucase & "' (empty = built in)" & chr(10) );
</cfscript>
