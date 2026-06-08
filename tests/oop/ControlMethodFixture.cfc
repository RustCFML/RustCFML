<!---
    Control fixture: a component with only ordinary (non-builtin-named) methods.
    Proves the fixture/createObject wiring is sound, so a failure on
    BuiltinNameMethodFixture isolates to the builtin-name collision.
--->
<cfcomponent output="false">
    <cffunction name="ping" access="public" returntype="string" output="false">
        <cfargument name="u" type="string" required="true" />
        <cfreturn "ping:" & arguments.u />
    </cffunction>
</cfcomponent>
