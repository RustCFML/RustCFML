<!---
    Gap fixture: an ESCAPED hash immediately followed by an interpolation
    inside a cfquery body's SQL string literal -- 'Order ###x#' (## is a
    literal hash, #x# interpolates). On Lucee the query yields "Order #5"
    when x=5. RustCFML fails to PARSE the body ("Expected RParen, found
    Identifier(\"x\")"), degrading this component to a catchable
    createObject() failure -- hence the runtime-instantiated fixture. The
    same ###x# sequence works in cfoutput/cfset string contexts on RustCFML
    (asserted inline in the test file); the gap is specific to the cfquery
    body lowering. QoQ (dbtype="query") keeps the fixture executable with no
    datasource on either engine.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset var x = 5>
        <cfset var src = queryNew("id", "integer", [[1]])>
        <cfquery name="local.q" dbtype="query">
            SELECT 'Order ###x#' AS t FROM src
        </cfquery>
        <cfreturn local.q.t>
    </cffunction>
</cfcomponent>
