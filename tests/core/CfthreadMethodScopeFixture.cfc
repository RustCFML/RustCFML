<cfcomponent>
    <!---
        A cfthread body spawned from inside a component method runs with that
        component's context: its methods (public AND private) resolve by bare
        name and through `variables.` exactly as they do in the method itself.
        Each leg returns "STATUS|out" (or "STATUS|THREW:message") so the caller
        asserts a value rather than an aborted request.
    --->

    <cffunction name="bareSiblingCall" access="public">
        <cfthread name="ctms_bare" x="5">
            <cfset thread.out = helper(attributes.x) />
        </cfthread>
        <cfthread action="join" name="ctms_bare" timeout="5000" />
        <cfreturn describe(cfthread.ctms_bare) />
    </cffunction>

    <cffunction name="variablesScopedCall" access="public">
        <cfthread name="ctms_vars" x="5">
            <cfset thread.out = variables.helper(attributes.x) />
        </cfthread>
        <cfthread action="join" name="ctms_vars" timeout="5000" />
        <cfreturn describe(cfthread.ctms_vars) />
    </cffunction>

    <cffunction name="bareSiblingCallNested" access="public">
        <cfthread name="ctms_nested" x="5">
            <cfset thread.out = helperNested(attributes.x) />
        </cfthread>
        <cfthread action="join" name="ctms_nested" timeout="5000" />
        <cfreturn describe(cfthread.ctms_nested) />
    </cffunction>

    <cffunction name="thisPassedAsAttribute" access="public">
        <cfthread name="ctms_this" x="5" self="#this#">
            <cfset thread.out = attributes.self.publicHelper(attributes.x) />
        </cfthread>
        <cfthread action="join" name="ctms_this" timeout="5000" />
        <cfreturn describe(cfthread.ctms_this) />
    </cffunction>

    <cffunction name="functionPassedAsAttribute" access="public">
        <cfset var fn = helperNested />
        <cfthread name="ctms_fnattr" x="5" fn="#fn#">
            <cfset local.worker = attributes.fn />
            <cfset thread.out = worker(attributes.x) />
        </cfthread>
        <cfthread action="join" name="ctms_fnattr" timeout="5000" />
        <cfreturn describe(cfthread.ctms_fnattr) />
    </cffunction>

    <cffunction name="publicHelper" access="public">
        <cfargument name="x">
        <cfreturn helper(arguments.x) />
    </cffunction>

    <cffunction name="helper" access="private">
        <cfargument name="x">
        <cfreturn arguments.x * 2 />
    </cffunction>

    <!--- 5 -> helper(5) + variables.helper(1) = 10 + 2 --->
    <cffunction name="helperNested" access="private">
        <cfargument name="x">
        <cfreturn helper(arguments.x) + variables.helper(1) />
    </cffunction>

    <cffunction name="describe" access="private">
        <cfargument name="t">
        <cfif structKeyExists(arguments.t, "out")>
            <cfreturn arguments.t.status & "|" & arguments.t.out />
        </cfif>
        <cfset var err = arguments.t.error ?: "" />
        <cfif isStruct(err)>
            <cfset err = err.message ?: "" />
        </cfif>
        <cfreturn arguments.t.status & "|THREW:" & listFirst(err, chr(10)) />
    </cffunction>
</cfcomponent>
