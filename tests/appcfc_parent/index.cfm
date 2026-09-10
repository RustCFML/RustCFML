<cfscript>
// Target page for oop/test_appcfc_extends_parent_methods.cfm.
writeOutput( ( structKeyExists( request, "bootParentPing" ) ? request.bootParentPing : "NO-PING" ) & "|" & ( structKeyExists( request, "bootParentVars" ) ? request.bootParentVars : "" ) );
</cfscript>
