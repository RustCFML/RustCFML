<!---
    Gap fixture: <cfhttp> with an interpolated timeout (timeout="#arguments.timeout#").
    Every other cfhttp string attribute (url, method, charset, ...) interpolates,
    but `timeout` was emitted verbatim, leaving literal "#...#" in the generated
    script ("timeout: #arguments.timeout#") so the component failed to PARSE. The
    failure surfaces at createObject() time as "Could not find the component",
    hence the runtime-instantiated fixture (an inline parse error escapes
    try/catch and would abort the runner). The request is guarded by <cfif false>
    so it is compiled but never executed.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfargument name="timeout" default="5" />
        <cfif false>
            <cfhttp url="http://127.0.0.1/probe" method="GET" result="local.r" timeout="#arguments.timeout#"></cfhttp>
        </cfif>
        <cfreturn "ok" />
    </cffunction>
</cfcomponent>
