<cfcomponent extends="BaseApplication" output="false">
    <cfset this.name = "rustcfml_lifecycle_case_override" />

    <cffunction name="onApplicationStart" returntype="boolean" output="false">
        <cfset super.onApplicationStart() />
        <cfset application.lifecycle_case_child = "child-ran" />
        <cfreturn true />
    </cffunction>
</cfcomponent>
