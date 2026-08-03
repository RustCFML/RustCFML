<cfif thisTag.executionMode EQ "start">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[start:" & attributes.label & "]">
    <cfset thisTag.carried = "S0">
</cfif>

<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.countVar] = caller[attributes.countVar] + 1>
    <cfset caller[attributes.outVar] = caller[attributes.outVar]
        & "[end" & caller[attributes.countVar]
        & " gc=" & trim(thisTag.generatedContent)
        & " carried=" & (structKeyExists(thisTag, "carried") ? thisTag.carried : "GONE")
        & " mode=" & thisTag.executionMode
        & " attr=" & attributes.label & "]">
    <cfset thisTag.carried = "S" & caller[attributes.countVar]>
    <cfset thisTag.generatedContent = "">
    <cfif caller[attributes.countVar] LT 3>
        <cfexit method="loop">
    </cfif>
</cfif>
