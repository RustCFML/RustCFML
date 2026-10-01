<cfcomponent output="false">
    <cfset this.name = "rustcfml_session_builtins" />
    <cfset this.sessionManagement = true />
    <cfset this.sessionTimeout = createTimespan(0, 0, 10, 0) />
</cfcomponent>
