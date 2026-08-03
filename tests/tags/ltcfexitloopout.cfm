<cfif thisTag.executionMode EQ "start">
    <cfset thisTag.carried = "S0">
    <!--- keep an alias: after the phase flip it must observe the CURRENT
          executionMode / generatedContent, not a stale snapshot --->
    <cfset caller[attributes.aliasVar] = thisTag>
</cfif>

<cfif thisTag.executionMode EQ "end">
    <cfset caller[attributes.countVar] = caller[attributes.countVar] + 1>
    <!--- deliberately does NOT clear generatedContent: the caller's page must
          receive every iteration's body output, in order, each followed by
          this tag's own end-phase output. --->
    <cfoutput>(e#caller[attributes.countVar]#)</cfoutput>
    <cfset thisTag.carried = "S" & caller[attributes.countVar]>
    <cfif caller[attributes.countVar] LT 3>
        <cfexit method="loop">
    </cfif>
</cfif>
