<cfcomponent extends="BaseApplication" output="false">
    <cfset this.name = "rustcfml_lifecycle_super_this" />

    <cffunction name="onApplicationStart" returntype="boolean" output="false">
        <cfset super.onApplicationStart() />
        <cfset application.lifecycle_super_this_child = "child-ran" />
        <cfreturn true />
    </cffunction>
</cfcomponent>
