<cfcomponent output="false">
    <cfset this.parentMarker = "parent-this" />

    <cffunction name="OnApplicationStart" returntype="boolean" output="false">
        <cfset application.lifecycle_super_this_parent = this.parentMarker />
        <cfreturn true />
    </cffunction>
</cfcomponent>
