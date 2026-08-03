<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[first-end]">
    <cfexit method="exittag">
</cfif>
