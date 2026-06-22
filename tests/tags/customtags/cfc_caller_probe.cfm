<cfif thisTag.executionMode EQ "end">
    <cfif structKeyExists(caller, "payload")>
        <cfoutput>caller:#caller.payload.label#|body:#trim(thisTag.generatedContent)#</cfoutput>
    <cfelse>
        <cfoutput>caller:missing|body:#trim(thisTag.generatedContent)#</cfoutput>
    </cfif>
    <cfset thisTag.generatedContent = "" />
</cfif>
