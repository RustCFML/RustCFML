<!---
    Control fixture: <cfhttp> with a literal numeric timeout (timeout="5").
    RustCFML already parses this, so it is the regression guard proving the
    cfhttp-in-fixture wiring is sound. The request is guarded by <cfif false>
    so it is compiled but never executed (no network call); this test is about
    PARSE-time handling of the timeout attribute, not delivery.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfargument name="timeout" default="5" />
        <cfif false>
            <cfhttp url="http://127.0.0.1/probe" method="GET" result="local.r" timeout="5"></cfhttp>
        </cfif>
        <cfreturn "ok" />
    </cffunction>
</cfcomponent>
