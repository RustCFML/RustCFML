<cfcomponent output="false" hint="Control: same shape with a method named form() -- the form scope is undisturbed on both engines.">
    <cffunction name="form" returntype="string" output="false">
        <cfreturn "fn-form" />
    </cffunction>
</cfcomponent>
