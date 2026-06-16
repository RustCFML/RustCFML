<!---
    Gap fixture: a tag-form <cftry>/<cffinally> with no <cfcatch>, written with
    normal multi-line formatting. Lucee accepts a catchless try/finally and runs
    the finally block on the normal path; RustCFML 0.191.0 rejects this shape at
    parse time with "Expected RBrace, found Semicolon". hello hits this in
    apps/hub/routes/_import/nsw_land_values.cfc and
    apps/www/routes/easy/[sell_id]/instant_offer/step1.cfc.
--->
<cfcomponent output="false">
    <cffunction name="run" returntype="string" output="false">
        <cfset var steps = [] />
        <cftry>
            <cfset arrayAppend(steps, "try") />
            <cffinally>
                <cfset arrayAppend(steps, "finally") />
            </cffinally>
        </cftry>
        <cfreturn arrayToList(steps) />
    </cffunction>
</cfcomponent>
