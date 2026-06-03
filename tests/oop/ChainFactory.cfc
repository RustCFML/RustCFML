<cfcomponent>
    <cffunction name="init">
        <cfset variables.kind = "factory">
        <cfreturn this>
    </cffunction>

    <cffunction name="kind">
        <cfreturn variables.kind>
    </cffunction>

    <cffunction name="getService">
        <cfargument name="table_name" default="moo_profile">
        <cfreturn createObject("component", "oop.ChainService").init()>
    </cffunction>
</cfcomponent>
