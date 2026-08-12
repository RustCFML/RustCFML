<!---
    Gap fixture: an ARROW FUNCTION with an expression body inside a TAG-MODE
    expression -- <cfset total = arr.reduce((s, r) => s + r.count, 0)>. Lucee
    evaluates it (run() returns 5). RustCFML fails to PARSE the tag-mode
    expression ("Expected RParen, found Comma" -- the parameter list is read
    as a parenthesised expression), degrading this component to a catchable
    createObject() failure -- hence the runtime-instantiated fixture. The
    IDENTICAL code inside <cfscript> works (inline control in the test file),
    as does the classic function(){} closure spelling in tag mode
    (TagModeArrowControlFixture).
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset var arr = [{count: 2}, {count: 3}]>
        <cfset var total = arr.reduce((s, r) => s + r.count, 0)>
        <cfreturn total>
    </cffunction>
</cfcomponent>
