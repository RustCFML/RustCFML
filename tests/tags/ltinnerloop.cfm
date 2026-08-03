<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.countVar] = caller[attributes.countVar] + 1>
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "(in" & caller[attributes.countVar] & ")">
    <cfif caller[attributes.countVar] MOD 2 EQ 1>
        <cfexit method="loop">
    </cfif>
</cfif>
