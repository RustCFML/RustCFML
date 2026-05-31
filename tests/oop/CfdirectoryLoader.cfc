<cfcomponent>

    <cffunction name="loadRoutes" returntype="string">
        <cfdirectory action="list" directory="/oop/native_cfcs" name="qRoutes" filter="*.cfc">
        <cfreturn qRoutes.recordCount & "|" & qRoutes.columnList>
    </cffunction>

</cfcomponent>
