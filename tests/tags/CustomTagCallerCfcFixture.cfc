<cfcomponent output="false">
    <cffunction name="render" returntype="string" output="false">
        <cfset payload = { label: "method-scope" } />
        <cfsavecontent variable="local.out"><cfmodule template="customtags/cfc_caller_probe.cfm">BODY</cfmodule></cfsavecontent>
        <cfreturn trim(local.out) />
    </cffunction>
</cfcomponent>
