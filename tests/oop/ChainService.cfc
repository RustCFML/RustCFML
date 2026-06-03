<cfcomponent>
    <cffunction name="init">
        <cfset variables.prefix = "service">
        <cfreturn this>
    </cffunction>

    <cffunction name="login">
        <cfargument name="profile_id" required="true">
        <cfargument name="stay_logged_in" default="false">

        <cfreturn variables.prefix & ":" & arguments.profile_id & ":" & arguments.stay_logged_in>
    </cffunction>
</cfcomponent>
