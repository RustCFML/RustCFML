<cfif thisTag.executionMode EQ "start">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[second-start]">
</cfif>

<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[second-end:" & trim(thisTag.generatedContent) & "]">
    <cfset thisTag.generatedContent = "">
</cfif>
