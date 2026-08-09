<!---
    Gap fixture: the BLOCK-BODY arrow spelling in a tag-mode expression --
    <cfset total = arr.reduce((s, r) => { return s + r.count; }, 0)>. Same
    parse failure as the expression-body form (TagModeArrowFixture):
    "Expected RParen, found Comma". Pinned separately so a fix for one body
    shape cannot leave the other behind.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset var arr = [{count: 2}, {count: 3}]>
        <cfset var total = arr.reduce((s, r) => { return s + r.count; }, 0)>
        <cfreturn total>
    </cffunction>
</cfcomponent>
