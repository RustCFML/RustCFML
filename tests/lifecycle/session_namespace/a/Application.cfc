<cfcomponent output="false">
    <cfset this.name = "lucee_compat_session_ns_a" />
    <cfset this.sessionManagement = true />
    <cfset this.sessionTimeout = createTimespan(0, 0, 10, 0) />

    <cffunction name="onSessionStart" output="false">
        <cfset session.started_in = "A" />
    </cffunction>
</cfcomponent>
