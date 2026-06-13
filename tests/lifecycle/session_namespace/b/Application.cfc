<cfcomponent output="false">
    <cfset this.name = "lucee_compat_session_ns_b" />
    <cfset this.sessionManagement = true />
    <cfset this.sessionTimeout = createTimespan(0, 0, 10, 0) />

    <cffunction name="onSessionStart" output="false">
        <cfset session.started_in = "B" />
    </cffunction>
</cfcomponent>
