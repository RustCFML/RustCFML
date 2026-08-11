<!---
    Control fixture: the classic function(){} closure spelling in the same
    tag-mode expression -- arr.reduce(function(s, r) { return s + r.count; }, 0).
    RustCFML already parses and evaluates this, so it guards the fixture
    wiring: only the ARROW spelling trips the gap fixtures.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset var arr = [{count: 2}, {count: 3}]>
        <cfset var total = arr.reduce(function(s, r) { return s + r.count; }, 0)>
        <cfreturn total>
    </cffunction>
</cfcomponent>
