<!---
    Gap fixture: a cfargument default= given as an UNQUOTED hash-wrapped
    struct literal -- default=#{ "type": "json_object" }#. Lucee/ACF evaluate
    the hash-wrapped expression and the paramless call t() returns
    "json_object". RustCFML fails to PARSE the attribute ("Unterminated '#'
    interpolation in string", reported at a position away from the offending
    attribute), degrading this component to a catchable createObject()
    failure -- hence the runtime-instantiated fixture.
--->
<cfcomponent output="false">
    <cffunction name="t" returntype="string" output="false">
        <cfargument name="rf" type="struct" default=#{ "type": "json_object" }#>
        <cfreturn arguments.rf.type>
    </cffunction>
    <cffunction name="run" returntype="string" output="false">
        <cfreturn t()>
    </cffunction>
</cfcomponent>
