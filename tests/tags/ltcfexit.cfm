<cfif thisTag.executionMode EQ "start">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[start]">
    <cfexit method="exittag">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[after-start]">
</cfif>

<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.outVar] = caller[attributes.outVar] & "[end:" & trim(thisTag.generatedContent) & "]">
    <cfset thisTag.generatedContent = "">
</cfif>
