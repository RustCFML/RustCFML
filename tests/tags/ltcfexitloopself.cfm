<cfif thisTag.executionMode EQ "start">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[start]">
    <cfset thisTag.carried = "S0">
</cfif>

<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.countVar] = caller[attributes.countVar] + 1>
    <cfset caller[attributes.outVar] = caller[attributes.outVar]
        & "[end" & caller[attributes.countVar]
        & " carried=" & (structKeyExists(thisTag, "carried") ? thisTag.carried : "GONE") & "]">
    <cfset thisTag.carried = "S" & caller[attributes.countVar]>
    <cfif caller[attributes.countVar] LT 3>
        <cfexit method="loop">
    </cfif>
</cfif>
