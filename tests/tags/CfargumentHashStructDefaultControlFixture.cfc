<!---
    Control fixture: an unquoted hash-wrapped SIMPLE expression as the
    cfargument default -- default=#lCase("JSON_OBJECT")#. RustCFML already
    parses and evaluates this, so it guards the fixture wiring: only the
    STRUCT-LITERAL body of the hash expression trips the gap fixture.
--->
<cfcomponent output="false">
    <cffunction name="t" returntype="string" output="false">
        <cfargument name="rf" type="string" default=#lCase("JSON_OBJECT")#>
        <cfreturn arguments.rf>
    </cffunction>
    <cffunction name="run" returntype="string" output="false">
        <cfreturn t()>
    </cffunction>
</cfcomponent>
