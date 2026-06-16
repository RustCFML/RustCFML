<cfif thisTag.executionMode EQ "start">
    <cfsleep time="#attributes.time#">
    <cfset caller[attributes.outVar] = "slept">
</cfif>
