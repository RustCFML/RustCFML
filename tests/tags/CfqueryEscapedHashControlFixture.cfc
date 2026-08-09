<!---
    Control fixture: plain interpolation (no escaped hash) in the same
    cfquery-body string literal -- 'Order #x#' yields "Order 5". RustCFML
    already parses and executes this, so it guards the QoQ-in-fixture wiring:
    only the ## escape immediately before the interpolation trips the gap
    fixture.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset var x = 5>
        <cfset var src = queryNew("id", "integer", [[1]])>
        <cfquery name="local.q" dbtype="query">
            SELECT 'Order #x#' AS t FROM src
        </cfquery>
        <cfreturn local.q.t>
    </cffunction>
</cfcomponent>
