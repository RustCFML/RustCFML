<cfcomponent output="true" hint="Same shape with output=true -- BOTH engines leak the inter-tag whitespace here, so the suppression above is the attribute doing its job and not a blanket trim.">
    <cfset this.a = 1 />
    <cfset this.b = 2 />
    <cffunction name="m" returntype="string" output="true">
        <cfset var q = 1 />
        <cfreturn "M" />
    </cffunction>
</cfcomponent>
