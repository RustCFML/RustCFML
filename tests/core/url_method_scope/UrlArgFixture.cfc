<cfcomponent output="false" hint="Control: an ARGUMENT named url is fine on both engines.">
    <cffunction name="go" returntype="string" output="false">
        <cfargument name="url" type="string" required="true" />
        <cfreturn "arg:" & arguments.url />
    </cffunction>
</cfcomponent>
