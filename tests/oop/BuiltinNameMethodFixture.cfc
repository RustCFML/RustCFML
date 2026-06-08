<!---
    Fixture: a component that defines methods whose names collide with built-in
    functions (canonicalize, reverse) alongside an ordinary method. On
    Lucee/ACF this is legal -- a method named like a builtin is callable via
    obj.method(); object method dispatch takes precedence over the builtin.

    On RustCFML v0.75.0+ (commit cc6279a, "Lucee-parity reject of builtin
    redefinition") the DefineFunction op throws / drops any non-"__" function
    whose name is a builtin, which over-reaches to COMPONENT METHODS -- so these
    methods do not register and obj.canonicalize()/obj.reverse() fail with
    "Component has no function with name [...]". The `plain` sibling is here to
    detect whether a builtin-named method poisons the whole component.
--->
<cfcomponent output="false">
    <cffunction name="canonicalize" access="public" returntype="string" output="false">
        <cfargument name="u" type="string" required="true" />
        <cfreturn "canon:" & arguments.u />
    </cffunction>

    <cffunction name="reverse" access="public" returntype="string" output="false">
        <cfargument name="u" type="string" required="true" />
        <cfreturn "rev:" & arguments.u />
    </cffunction>

    <cffunction name="plain" access="public" returntype="string" output="false">
        <cfargument name="u" type="string" required="true" />
        <cfreturn "plain:" & arguments.u />
    </cffunction>
</cfcomponent>
