<cfcomponent output="false">
    <cffunction name="OnApplicationStart" returntype="boolean" output="false">
        <cfset application.lifecycle_case_parent = "parent-ran" />
        <cfreturn true />
    </cffunction>
</cfcomponent>
