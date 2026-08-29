<cfcomponent output="false" hint="Tag-based CFC whose pseudo-constructor must emit NOTHING -- output=false on the component covers the body the same way it covers a method.">
    <cfset this.a = 1 />
    <cfset this.b = 2 />
    <cfoutput>PSEUDO</cfoutput>
    <cffunction name="m" returntype="string" output="false">
        <cfset var q = 1 />
        <cfreturn "M" />
    </cffunction>
</cfcomponent>
